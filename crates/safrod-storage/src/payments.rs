use safrod_core::ids::PaymentId;
use safrod_core::state::PaymentStatus;
use serde_json::{Value, json};

use crate::events::{claim_node_event, emit};
use crate::models::PaymentRecord;
use crate::{Db, StorageError, to_i64};

pub struct NewPayment {
    pub id: PaymentId,
    pub project_id: String,
    pub idempotency_key: String,
    pub request_hash: Vec<u8>,
    pub bolt11: String,
    pub payment_hash: String,
    pub amount_msat: u64,
    pub metadata: Value,
}

pub struct BeginOutcome {
    pub payment: PaymentRecord,
    /// False when the idempotency key already existed. The caller must then
    /// return the stored payment and must not dispatch a second payment.
    pub created: bool,
}

#[derive(Default)]
pub struct Resolution {
    pub fee_msat: Option<u64>,
    pub failure_reason: Option<String>,
    pub preimage: Option<String>,
}

pub struct Transitioned {
    pub payment: PaymentRecord,
    pub changed: bool,
}

pub fn payment_event_data(payment: &PaymentRecord) -> Value {
    json!({
        "payment_id": payment.id,
        "status": payment.status,
        "amount_msat": payment.amount_msat,
        "fee_msat": payment.fee_msat,
        "payment_hash": payment.payment_hash,
        "failure_reason": payment.failure_reason,
        "real_regtest_lightning": true,
    })
}

impl Db {
    pub async fn begin_payment(&self, new: NewPayment) -> Result<BeginOutcome, StorageError> {
        let mut tx = self.pool.begin().await?;
        let inserted: Option<PaymentRecord> = sqlx::query_as(
            "insert into outgoing_payments
               (id, project_id, idempotency_key, request_hash, bolt11, payment_hash,
                amount_msat, status, metadata)
             values ($1, $2, $3, $4, $5, $6, $7, 'created', $8)
             on conflict (project_id, idempotency_key) do nothing
             returning id, project_id, idempotency_key, bolt11, payment_hash, amount_msat,
                       fee_msat, status, failure_reason, preimage, metadata, created_at, updated_at",
        )
        .bind(new.id.as_str())
        .bind(&new.project_id)
        .bind(&new.idempotency_key)
        .bind(&new.request_hash)
        .bind(&new.bolt11)
        .bind(&new.payment_hash)
        .bind(to_i64(new.amount_msat)?)
        .bind(&new.metadata)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(payment) = inserted {
            tx.commit().await?;
            return Ok(BeginOutcome {
                payment,
                created: true,
            });
        }
        let (existing_hash,): (Vec<u8>,) = sqlx::query_as(
            "select request_hash from outgoing_payments where project_id = $1 and idempotency_key = $2",
        )
        .bind(&new.project_id)
        .bind(&new.idempotency_key)
        .fetch_one(&mut *tx)
        .await?;
        if existing_hash != new.request_hash {
            return Err(StorageError::IdempotencyMismatch);
        }
        let payment = sqlx::query_as(
            "select id, project_id, idempotency_key, bolt11, payment_hash, amount_msat,
                    fee_msat, status, failure_reason, preimage, metadata, created_at, updated_at
             from outgoing_payments where project_id = $1 and idempotency_key = $2",
        )
        .bind(&new.project_id)
        .bind(&new.idempotency_key)
        .fetch_one(&mut *tx)
        .await?;
        Ok(BeginOutcome {
            payment,
            created: false,
        })
    }

    pub async fn get_payment(
        &self,
        project_id: &str,
        id: &str,
    ) -> Result<PaymentRecord, StorageError> {
        Ok(sqlx::query_as(PAYMENT_SELECT_BY_ID)
            .bind(project_id)
            .bind(id)
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn list_payments(
        &self,
        project_id: &str,
        starting_after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<PaymentRecord>, StorageError> {
        Ok(sqlx::query_as(
            "select id, project_id, idempotency_key, bolt11, payment_hash, amount_msat,
                    fee_msat, status, failure_reason, preimage, metadata, created_at, updated_at
             from outgoing_payments
             where project_id = $1
               and ($2::text is null or (created_at, id) <
                    (select created_at, id from outgoing_payments where id = $2 and project_id = $1))
             order by created_at desc, id desc limit $3",
        )
        .bind(project_id)
        .bind(starting_after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Payments that are not final. Used after a restart to reconcile with
    /// the node.
    pub async fn unresolved_payments(&self) -> Result<Vec<PaymentRecord>, StorageError> {
        Ok(sqlx::query_as(
            "select id, project_id, idempotency_key, bolt11, payment_hash, amount_msat,
                    fee_msat, status, failure_reason, preimage, metadata, created_at, updated_at
             from outgoing_payments where status in ('created', 'pending', 'unknown')
             order by created_at",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn find_unresolved_payment_by_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<PaymentRecord>, StorageError> {
        Ok(sqlx::query_as(
            "select id, project_id, idempotency_key, bolt11, payment_hash, amount_msat,
                    fee_msat, status, failure_reason, preimage, metadata, created_at, updated_at
             from outgoing_payments
             where payment_hash = $1 and status in ('created', 'pending', 'unknown')
             order by created_at desc limit 1",
        )
        .bind(payment_hash)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Moves a payment to `next`. Repeating the current status is a no-op, so
    /// duplicated node events and reconciliation passes are safe. Moving out of
    /// a final status is refused.
    pub async fn transition_payment(
        &self,
        dedupe_key: Option<&str>,
        payment_id: &str,
        next: PaymentStatus,
        resolution: Resolution,
    ) -> Result<Transitioned, StorageError> {
        let mut tx = self.pool.begin().await?;
        let current: PaymentRecord = sqlx::query_as(
            "select id, project_id, idempotency_key, bolt11, payment_hash, amount_msat,
                    fee_msat, status, failure_reason, preimage, metadata, created_at, updated_at
             from outgoing_payments where id = $1 for update",
        )
        .bind(payment_id)
        .fetch_one(&mut *tx)
        .await?;
        if let Some(key) = dedupe_key {
            let payload = json!({ "payment_id": payment_id, "status": next.as_str() });
            if !claim_node_event(&mut tx, key, "payment_outcome", &payload).await? {
                return Ok(Transitioned {
                    payment: current,
                    changed: false,
                });
            }
        }
        let from: PaymentStatus = current.status.parse().map_err(|_| StorageError::NotFound)?;
        if from == next {
            return Ok(Transitioned {
                payment: current,
                changed: false,
            });
        }
        from.transition(next)
            .map_err(|_| StorageError::IllegalTransition {
                from: from.as_str(),
                to: next.as_str(),
            })?;
        let fee = resolution.fee_msat.map(to_i64).transpose()?;
        let updated: PaymentRecord = sqlx::query_as(
            "update outgoing_payments set status = $2, updated_at = now(),
                    fee_msat = coalesce($3, fee_msat),
                    failure_reason = coalesce($4, failure_reason),
                    preimage = coalesce($5, preimage)
             where id = $1
             returning id, project_id, idempotency_key, bolt11, payment_hash, amount_msat,
                       fee_msat, status, failure_reason, preimage, metadata, created_at, updated_at",
        )
        .bind(payment_id)
        .bind(next.as_str())
        .bind(fee)
        .bind(&resolution.failure_reason)
        .bind(&resolution.preimage)
        .fetch_one(&mut *tx)
        .await?;
        if matches!(
            next,
            PaymentStatus::Succeeded | PaymentStatus::Failed | PaymentStatus::Unknown
        ) {
            emit(
                &mut tx,
                &updated.project_id,
                &format!("payment.{}", next.as_str()),
                &updated.id,
                payment_event_data(&updated),
                false,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(Transitioned {
            payment: updated,
            changed: true,
        })
    }
}

const PAYMENT_SELECT_BY_ID: &str =
    "select id, project_id, idempotency_key, bolt11, payment_hash, amount_msat,
        fee_msat, status, failure_reason, preimage, metadata, created_at, updated_at
 from outgoing_payments where project_id = $1 and id = $2";
