use chrono::{DateTime, Utc};
use safrod_core::ids::{PayoutId, QuoteId};
use safrod_core::state::PayoutStatus;
use serde_json::json;

use crate::events::emit;
use crate::models::{PayoutRecord, QuoteRecord};
use crate::{Db, StorageError, to_i64};

pub struct NewQuote {
    pub project_id: String,
    pub source_currency: String,
    pub target_currency: String,
    pub source_minor: u64,
    pub target_minor: u64,
    pub rate_ppm: u64,
    pub btc_amount_msat: u64,
    pub expires_at: DateTime<Utc>,
}

pub fn payout_event_data(payout: &PayoutRecord) -> serde_json::Value {
    json!({
        "payout_id": payout.id,
        "quote_id": payout.quote_id,
        "status": payout.status,
        "scenario": payout.scenario,
        "payment_id": payout.payment_id,
        "simulated_fiat": true,
    })
}

impl Db {
    pub async fn insert_quote(&self, new: NewQuote) -> Result<QuoteRecord, StorageError> {
        Ok(sqlx::query_as(
            "insert into sandbox_quotes
               (id, project_id, source_currency, target_currency, source_minor, target_minor,
                rate_ppm, btc_amount_msat, status, expires_at)
             values ($1, $2, $3, $4, $5, $6, $7, $8, 'open', $9) returning *",
        )
        .bind(QuoteId::new().as_str())
        .bind(&new.project_id)
        .bind(&new.source_currency)
        .bind(&new.target_currency)
        .bind(to_i64(new.source_minor)?)
        .bind(to_i64(new.target_minor)?)
        .bind(to_i64(new.rate_ppm)?)
        .bind(to_i64(new.btc_amount_msat)?)
        .bind(new.expires_at)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_quote(&self, project_id: &str, id: &str) -> Result<QuoteRecord, StorageError> {
        Ok(
            sqlx::query_as("select * from sandbox_quotes where project_id = $1 and id = $2")
                .bind(project_id)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Reserves simulated liquidity for a quote: marks it used, debits the
    /// simulated source balance, and records the payout, all in one
    /// transaction. A stale or already-used quote is refused. Returns
    /// `(payout, created)`; `created` is false for an idempotent replay.
    pub async fn reserve_payout(
        &self,
        project_id: &str,
        quote_id: &str,
        idempotency_key: &str,
        scenario: &str,
    ) -> Result<(PayoutRecord, bool), StorageError> {
        let mut tx = self.pool.begin().await?;
        let prior: Option<PayoutRecord> = sqlx::query_as(
            "select * from sandbox_payouts where project_id = $1 and idempotency_key = $2",
        )
        .bind(project_id)
        .bind(idempotency_key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(prior) = prior {
            if prior.quote_id != quote_id || prior.scenario != scenario {
                return Err(StorageError::IdempotencyMismatch);
            }
            return Ok((prior, false));
        }
        let quote: QuoteRecord = sqlx::query_as(
            "select * from sandbox_quotes where project_id = $1 and id = $2 for update",
        )
        .bind(project_id)
        .bind(quote_id)
        .fetch_one(&mut *tx)
        .await?;
        if quote.status != "open" || quote.expires_at < Utc::now() {
            return Err(StorageError::IllegalTransition {
                from: "stale_quote",
                to: "reserved",
            });
        }
        sqlx::query("update sandbox_quotes set status = 'used' where id = $1")
            .bind(&quote.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "insert into sandbox_balances (project_id, currency, minor) values ($1, $2, $3)
             on conflict (project_id, currency) do update set minor = sandbox_balances.minor + $3",
        )
        .bind(project_id)
        .bind(&quote.source_currency)
        .bind(-quote.source_minor)
        .execute(&mut *tx)
        .await?;
        let payout: PayoutRecord = sqlx::query_as(
            "insert into sandbox_payouts (id, project_id, quote_id, idempotency_key, scenario, status)
             values ($1, $2, $3, $4, $5, 'reserved') returning *",
        )
        .bind(PayoutId::new().as_str())
        .bind(project_id)
        .bind(quote_id)
        .bind(idempotency_key)
        .bind(scenario)
        .fetch_one(&mut *tx)
        .await?;
        emit(
            &mut tx,
            project_id,
            "sandbox.payout.reserved",
            &payout.id,
            payout_event_data(&payout),
            true,
        )
        .await?;
        tx.commit().await?;
        Ok((payout, true))
    }

    pub async fn get_payout(
        &self,
        project_id: &str,
        id: &str,
    ) -> Result<PayoutRecord, StorageError> {
        Ok(
            sqlx::query_as("select * from sandbox_payouts where project_id = $1 and id = $2")
                .bind(project_id)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn attach_payment_to_payout(
        &self,
        payout_id: &str,
        payment_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("update sandbox_payouts set payment_id = $2, updated_at = now() where id = $1")
            .bind(payout_id)
            .bind(payment_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Advances a simulated payout. A failed payout credits the reserved
    /// source amount back. Repeating the current status is a no-op.
    pub async fn advance_payout(
        &self,
        payout_id: &str,
        next: PayoutStatus,
    ) -> Result<(PayoutRecord, bool), StorageError> {
        let mut tx = self.pool.begin().await?;
        let current: PayoutRecord =
            sqlx::query_as("select * from sandbox_payouts where id = $1 for update")
                .bind(payout_id)
                .fetch_one(&mut *tx)
                .await?;
        let from: PayoutStatus = current.status.parse().map_err(|_| StorageError::NotFound)?;
        if from == next {
            return Ok((current, false));
        }
        if !from.can_transition_to(next) {
            return Err(StorageError::IllegalTransition {
                from: from.as_str(),
                to: next.as_str(),
            });
        }
        let updated: PayoutRecord = sqlx::query_as(
            "update sandbox_payouts set status = $2, updated_at = now() where id = $1 returning *",
        )
        .bind(payout_id)
        .bind(next.as_str())
        .fetch_one(&mut *tx)
        .await?;
        if next == PayoutStatus::Failed {
            sqlx::query(
                "update sandbox_balances b set minor = b.minor + q.source_minor
                 from sandbox_quotes q
                 where q.id = $1 and b.project_id = q.project_id and b.currency = q.source_currency",
            )
            .bind(&updated.quote_id)
            .execute(&mut *tx)
            .await?;
        }
        emit(
            &mut tx,
            &updated.project_id,
            &format!("sandbox.payout.{}", next.as_str()),
            &updated.id,
            payout_event_data(&updated),
            true,
        )
        .await?;
        tx.commit().await?;
        Ok((updated, true))
    }

    pub async fn sandbox_balances(
        &self,
        project_id: &str,
    ) -> Result<Vec<(String, i64)>, StorageError> {
        Ok(sqlx::query_as(
            "select currency, minor from sandbox_balances where project_id = $1 order by currency",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn credit_sandbox_balance(
        &self,
        project_id: &str,
        currency: &str,
        minor: u64,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "insert into sandbox_balances (project_id, currency, minor) values ($1, $2, $3)
             on conflict (project_id, currency) do update set minor = sandbox_balances.minor + $3",
        )
        .bind(project_id)
        .bind(currency)
        .bind(to_i64(minor)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
