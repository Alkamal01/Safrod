use chrono::{DateTime, Utc};
use safrod_core::ids::EventId;
use serde_json::Value;
use sqlx::{Postgres, Transaction};

use crate::models::EventRecord;
use crate::{Db, StorageError};

/// Writes an event and one pending delivery per enabled endpoint in the
/// caller's transaction. A state change and its notification therefore commit
/// or roll back together; a crash can never leave a paid invoice without its
/// webhook, or a webhook for an invoice that is not paid.
pub(crate) async fn emit(
    tx: &mut Transaction<'_, Postgres>,
    project_id: &str,
    kind: &str,
    resource_id: &str,
    data: Value,
    simulated: bool,
) -> Result<EventRecord, StorageError> {
    let event: EventRecord = sqlx::query_as(
        "insert into events (id, project_id, type, resource_id, data, simulated)
         values ($1, $2, $3, $4, $5, $6) returning *",
    )
    .bind(EventId::new().as_str())
    .bind(project_id)
    .bind(kind)
    .bind(resource_id)
    .bind(data)
    .bind(simulated)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        "insert into webhook_deliveries (id, event_seq, endpoint_id, status)
         select 'dlv_' || replace(gen_random_uuid()::text, '-', ''), $1, id, 'pending'
         from webhook_endpoints where project_id = $2 and enabled",
    )
    .bind(event.seq)
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    Ok(event)
}

/// Records a node event under its dedupe key. Returns false if it was already
/// applied, in which case the caller must not apply it again.
pub(crate) async fn claim_node_event(
    tx: &mut Transaction<'_, Postgres>,
    dedupe_key: &str,
    kind: &str,
    payload: &Value,
) -> Result<bool, StorageError> {
    let done = sqlx::query(
        "insert into node_events (dedupe_key, kind, payload) values ($1, $2, $3)
         on conflict (dedupe_key) do nothing",
    )
    .bind(dedupe_key)
    .bind(kind)
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(done.rows_affected() == 1)
}

impl Db {
    pub async fn list_events(
        &self,
        project_id: &str,
        after_seq: Option<i64>,
        kind: Option<&str>,
        limit: i64,
    ) -> Result<Vec<EventRecord>, StorageError> {
        Ok(sqlx::query_as(
            "select * from events
             where project_id = $1 and seq > $2 and ($3::text is null or type = $3)
             order by seq asc limit $4",
        )
        .bind(project_id)
        .bind(after_seq.unwrap_or(0))
        .bind(kind)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_event(&self, project_id: &str, id: &str) -> Result<EventRecord, StorageError> {
        Ok(
            sqlx::query_as("select * from events where project_id = $1 and id = $2")
                .bind(project_id)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Emits an event outside any other state change, for sandbox and
    /// operator-triggered notifications.
    pub async fn emit_standalone(
        &self,
        project_id: &str,
        kind: &str,
        resource_id: &str,
        data: Value,
        simulated: bool,
    ) -> Result<EventRecord, StorageError> {
        let mut tx = self.pool.begin().await?;
        let event = emit(&mut tx, project_id, kind, resource_id, data, simulated).await?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn node_event_count(&self) -> Result<i64, StorageError> {
        let (n,): (i64,) = sqlx::query_as("select count(*) from node_events")
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }
}

pub fn event_envelope(
    id: &str,
    kind: &str,
    created_at: DateTime<Utc>,
    simulated: bool,
    data: &Value,
) -> Value {
    serde_json::json!({
        "id": id,
        "type": kind,
        "created_at": created_at,
        "simulated": simulated,
        "data": data,
    })
}
