use chrono::{DateTime, Utc};
use safrod_core::ids::InvoiceId;
use safrod_core::state::InvoiceStatus;
use serde_json::{Value, json};

use crate::events::{claim_node_event, emit};
use crate::models::InvoiceRecord;
use crate::{Db, StorageError, to_i64};

pub struct NewInvoice {
    pub id: InvoiceId,
    pub project_id: String,
    pub idempotency_key: Option<String>,
    pub amount_msat: u64,
    pub description: String,
    pub bolt11: String,
    pub payment_hash: String,
    pub expires_at: DateTime<Utc>,
    pub metadata: Value,
}

pub fn invoice_event_data(invoice: &InvoiceRecord) -> Value {
    json!({
        "invoice_id": invoice.id,
        "status": invoice.status,
        "amount_msat": invoice.amount_msat,
        "amount_received_msat": invoice.amount_received_msat,
        "payment_hash": invoice.payment_hash,
        "paid_at": invoice.paid_at,
        "real_regtest_lightning": true,
    })
}

impl Db {
    pub async fn insert_invoice(&self, new: NewInvoice) -> Result<InvoiceRecord, StorageError> {
        Ok(sqlx::query_as(
            "insert into invoices
               (id, project_id, idempotency_key, amount_msat, description, bolt11,
                payment_hash, status, expires_at, metadata)
             values ($1, $2, $3, $4, $5, $6, $7, 'open', $8, $9)
             returning *",
        )
        .bind(new.id.as_str())
        .bind(&new.project_id)
        .bind(&new.idempotency_key)
        .bind(to_i64(new.amount_msat)?)
        .bind(&new.description)
        .bind(&new.bolt11)
        .bind(&new.payment_hash)
        .bind(new.expires_at)
        .bind(&new.metadata)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_invoice(
        &self,
        project_id: &str,
        id: &str,
    ) -> Result<InvoiceRecord, StorageError> {
        Ok(
            sqlx::query_as("select * from invoices where project_id = $1 and id = $2")
                .bind(project_id)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn find_invoice_by_idempotency_key(
        &self,
        project_id: &str,
        key: &str,
    ) -> Result<Option<InvoiceRecord>, StorageError> {
        Ok(
            sqlx::query_as("select * from invoices where project_id = $1 and idempotency_key = $2")
                .bind(project_id)
                .bind(key)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn list_invoices(
        &self,
        project_id: &str,
        starting_after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<InvoiceRecord>, StorageError> {
        Ok(sqlx::query_as(
            "select * from invoices
             where project_id = $1
               and ($2::text is null or (created_at, id) <
                    (select created_at, id from invoices where id = $2 and project_id = $1))
             order by created_at desc, id desc limit $3",
        )
        .bind(project_id)
        .bind(starting_after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Applies a `PaymentReceived` node event. Returns `None` when the event
    /// was already applied or the invoice was already paid, so redelivery by
    /// the node is harmless.
    pub async fn mark_invoice_paid(
        &self,
        dedupe_key: &str,
        payment_hash: &str,
        amount_received_msat: u64,
    ) -> Result<Option<InvoiceRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let payload = json!({ "payment_hash": payment_hash, "amount_msat": amount_received_msat });
        if !claim_node_event(&mut tx, dedupe_key, "payment_received", &payload).await? {
            return Ok(None);
        }
        let current: InvoiceRecord =
            sqlx::query_as("select * from invoices where payment_hash = $1 for update")
                .bind(payment_hash)
                .fetch_one(&mut *tx)
                .await?;
        let status: InvoiceStatus = current.status.parse().map_err(|_| StorageError::NotFound)?;
        if status == InvoiceStatus::Paid {
            return Ok(None);
        }
        if !status.can_transition_to(InvoiceStatus::Paid) {
            return Err(StorageError::IllegalTransition {
                from: status.as_str(),
                to: InvoiceStatus::Paid.as_str(),
            });
        }
        let paid: InvoiceRecord = sqlx::query_as(
            "update invoices set status = 'paid', paid_at = now(), updated_at = now(),
                    amount_received_msat = $2
             where id = $1 returning *",
        )
        .bind(&current.id)
        .bind(to_i64(amount_received_msat)?)
        .fetch_one(&mut *tx)
        .await?;
        emit(
            &mut tx,
            &paid.project_id,
            "invoice.paid",
            &paid.id,
            invoice_event_data(&paid),
            false,
        )
        .await?;
        tx.commit().await?;
        Ok(Some(paid))
    }

    pub async fn expire_invoices(&self) -> Result<usize, StorageError> {
        let mut tx = self.pool.begin().await?;
        let expired: Vec<InvoiceRecord> = sqlx::query_as(
            "update invoices set status = 'expired', updated_at = now()
             where status = 'open' and expires_at < now() returning *",
        )
        .fetch_all(&mut *tx)
        .await?;
        for invoice in &expired {
            emit(
                &mut tx,
                &invoice.project_id,
                "invoice.expired",
                &invoice.id,
                invoice_event_data(invoice),
                false,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(expired.len())
    }
}
