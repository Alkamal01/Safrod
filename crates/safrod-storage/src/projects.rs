use safrod_core::apikey::GeneratedKey;
use safrod_core::ids::{ApiKeyId, ProjectId};

use crate::models::{ApiKeyRecord, AuthContext, Project};
use crate::{Db, StorageError};

impl Db {
    pub async fn create_project(&self, name: &str, network: &str) -> Result<Project, StorageError> {
        let project = sqlx::query_as::<_, Project>(
            "insert into projects (id, name, network) values ($1, $2, $3) returning *",
        )
        .bind(ProjectId::new().as_str())
        .bind(name)
        .bind(network)
        .fetch_one(&self.pool)
        .await?;
        Ok(project)
    }

    pub async fn get_project(&self, id: &str) -> Result<Project, StorageError> {
        Ok(sqlx::query_as("select * from projects where id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn create_api_key(
        &self,
        project_id: &str,
        label: &str,
        scopes: &[String],
        key: &GeneratedKey,
    ) -> Result<ApiKeyRecord, StorageError> {
        Ok(sqlx::query_as::<_, ApiKeyRecord>(
            "insert into api_keys (id, project_id, label, prefix, key_hash, scopes)
             values ($1, $2, $3, $4, $5, $6)
             returning id, project_id, label, prefix, scopes, revoked_at, created_at",
        )
        .bind(ApiKeyId::new().as_str())
        .bind(project_id)
        .bind(label)
        .bind(&key.prefix)
        .bind(&key.hash)
        .bind(scopes)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_api_keys(&self, project_id: &str) -> Result<Vec<ApiKeyRecord>, StorageError> {
        Ok(sqlx::query_as(
            "select id, project_id, label, prefix, scopes, revoked_at, created_at
             from api_keys where project_id = $1 order by created_at desc",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn revoke_api_key(&self, project_id: &str, key_id: &str) -> Result<(), StorageError> {
        let done = sqlx::query(
            "update api_keys set revoked_at = coalesce(revoked_at, now())
             where id = $1 and project_id = $2",
        )
        .bind(key_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    /// Looks a key up by digest. The unique index makes this a point lookup;
    /// the comparison itself is on the stored digest, never the secret.
    pub async fn authenticate(&self, key_hash: &[u8]) -> Result<Option<AuthContext>, StorageError> {
        let row: Option<(String, String, Vec<String>)> = sqlx::query_as(
            "select project_id, id, scopes from api_keys
             where key_hash = $1 and revoked_at is null",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(project_id, key_id, scopes)| AuthContext {
            project_id,
            key_id,
            scopes,
        }))
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, sqlx::FromRow)]
pub struct Totals {
    pub lightning_received_msat: i64,
    pub lightning_sent_msat: i64,
    pub lightning_fees_msat: i64,
    pub lightning_in_flight_msat: i64,
    pub onchain_confirmed_sat: i64,
    pub onchain_unconfirmed_sat: i64,
}

impl Db {
    /// Ledger totals derived from settled records. These are what Safrod has
    /// recorded for the project, not node balances.
    pub async fn project_totals(&self, project_id: &str) -> Result<Totals, StorageError> {
        Ok(sqlx::query_as(
            "select
               coalesce((select sum(amount_received_msat) from invoices
                         where project_id = $1 and status = 'paid'), 0)::bigint as lightning_received_msat,
               coalesce((select sum(amount_msat) from outgoing_payments
                         where project_id = $1 and status = 'succeeded'), 0)::bigint as lightning_sent_msat,
               coalesce((select sum(fee_msat) from outgoing_payments
                         where project_id = $1 and status = 'succeeded'), 0)::bigint as lightning_fees_msat,
               coalesce((select sum(amount_msat) from outgoing_payments
                         where project_id = $1 and status in ('pending', 'unknown')), 0)::bigint as lightning_in_flight_msat,
               coalesce((select sum(amount_sat) from onchain_transactions
                         where project_id = $1 and status = 'confirmed'), 0)::bigint as onchain_confirmed_sat,
               coalesce((select sum(amount_sat) from onchain_transactions
                         where project_id = $1 and status <> 'confirmed'), 0)::bigint as onchain_unconfirmed_sat",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?)
    }
}
