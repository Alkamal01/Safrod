use chrono::{DateTime, Utc};
use safrod_core::ids::WebhookEndpointId;

use crate::models::{DeliveryRecord, DueDelivery, WebhookEndpoint};
use crate::{Db, StorageError};

pub enum DeliveryOutcome {
    Delivered {
        status: i32,
    },
    Retry {
        status: Option<i32>,
        error: String,
        next_attempt_at: DateTime<Utc>,
    },
    Failed {
        status: Option<i32>,
        error: String,
    },
}

impl Db {
    pub async fn create_webhook_endpoint(
        &self,
        project_id: &str,
        url: &str,
        secret: &str,
    ) -> Result<WebhookEndpoint, StorageError> {
        Ok(sqlx::query_as(
            "insert into webhook_endpoints (id, project_id, url, secret) values ($1, $2, $3, $4)
             returning *",
        )
        .bind(WebhookEndpointId::new().as_str())
        .bind(project_id)
        .bind(url)
        .bind(secret)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_webhook_endpoints(
        &self,
        project_id: &str,
    ) -> Result<Vec<WebhookEndpoint>, StorageError> {
        Ok(sqlx::query_as(
            "select * from webhook_endpoints where project_id = $1 order by created_at desc",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn disable_webhook_endpoint(
        &self,
        project_id: &str,
        id: &str,
    ) -> Result<(), StorageError> {
        let done = sqlx::query(
            "update webhook_endpoints set enabled = false where id = $1 and project_id = $2",
        )
        .bind(id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    /// Claims deliveries that are due and leases them by pushing
    /// `next_attempt_at` forward, so two workers never send the same delivery
    /// and a crashed worker's claims become due again after the lease.
    pub async fn claim_due_deliveries(
        &self,
        limit: i64,
        lease_secs: i64,
    ) -> Result<Vec<DueDelivery>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let ids: Vec<(String,)> = sqlx::query_as(
            "select d.id from webhook_deliveries d
             join webhook_endpoints e on e.id = d.endpoint_id
             where d.status = 'pending' and d.next_attempt_at <= now() and e.enabled
             order by d.next_attempt_at limit $1 for update of d skip locked",
        )
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        let ids: Vec<String> = ids.into_iter().map(|(id,)| id).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query(
            "update webhook_deliveries
             set next_attempt_at = now() + make_interval(secs => $2)
             where id = any($1)",
        )
        .bind(&ids)
        .bind(lease_secs as f64)
        .execute(&mut *tx)
        .await?;
        let due = sqlx::query_as(
            "select d.id, d.attempts, e.url, e.secret,
                    ev.id as event_id, ev.type as event_type, ev.resource_id, ev.data,
                    ev.simulated, ev.created_at as event_created_at
             from webhook_deliveries d
             join webhook_endpoints e on e.id = d.endpoint_id
             join events ev on ev.seq = d.event_seq
             where d.id = any($1) order by ev.seq",
        )
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(due)
    }

    pub async fn record_delivery(
        &self,
        id: &str,
        outcome: DeliveryOutcome,
    ) -> Result<(), StorageError> {
        match outcome {
            DeliveryOutcome::Delivered { status } => {
                sqlx::query(
                    "update webhook_deliveries set status = 'delivered', attempts = attempts + 1,
                            last_status = $2, last_error = null, delivered_at = now()
                     where id = $1",
                )
                .bind(id)
                .bind(status)
                .execute(&self.pool)
                .await?;
            }
            DeliveryOutcome::Retry {
                status,
                error,
                next_attempt_at,
            } => {
                sqlx::query(
                    "update webhook_deliveries set attempts = attempts + 1, last_status = $2,
                            last_error = $3, next_attempt_at = $4
                     where id = $1",
                )
                .bind(id)
                .bind(status)
                .bind(error)
                .bind(next_attempt_at)
                .execute(&self.pool)
                .await?;
            }
            DeliveryOutcome::Failed { status, error } => {
                sqlx::query(
                    "update webhook_deliveries set status = 'failed', attempts = attempts + 1,
                            last_status = $2, last_error = $3
                     where id = $1",
                )
                .bind(id)
                .bind(status)
                .bind(error)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn list_deliveries(
        &self,
        project_id: &str,
        limit: i64,
    ) -> Result<Vec<DeliveryRecord>, StorageError> {
        Ok(sqlx::query_as(
            "select d.* from webhook_deliveries d
             join webhook_endpoints e on e.id = d.endpoint_id
             where e.project_id = $1 order by d.created_at desc, d.id desc limit $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Re-queues a delivery. Receivers deduplicate on the event id, so replay
    /// is safe by contract.
    pub async fn replay_delivery(&self, project_id: &str, id: &str) -> Result<(), StorageError> {
        let done = sqlx::query(
            "update webhook_deliveries d set status = 'pending', next_attempt_at = now()
             from webhook_endpoints e
             where d.id = $1 and e.id = d.endpoint_id and e.project_id = $2",
        )
        .bind(id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }
}
