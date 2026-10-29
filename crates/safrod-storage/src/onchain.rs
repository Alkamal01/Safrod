use safrod_core::ids::{AddressId, TransactionId};
use safrod_core::state::OnchainStatus;
use serde_json::{Value, json};

use crate::events::emit;
use crate::models::{AddressRecord, OnchainTransactionRecord};
use crate::{Db, StorageError, to_i64};

pub fn transaction_event_data(tx: &OnchainTransactionRecord, address: &str) -> Value {
    json!({
        "transaction_id": tx.id,
        "address": address,
        "txid": tx.txid,
        "vout": tx.vout,
        "amount_sat": tx.amount_sat,
        "confirmations": tx.confirmations,
        "block_height": tx.block_height,
        "status": tx.status,
    })
}

pub struct Observation<'a> {
    pub address: &'a AddressRecord,
    pub txid: &'a str,
    pub vout: i32,
    pub amount_sat: u64,
    pub block_height: Option<i32>,
    pub confirmations: i32,
}

impl Db {
    pub async fn create_address(
        &self,
        project_id: &str,
        address: &str,
        label: &str,
        required_confirmations: i32,
    ) -> Result<AddressRecord, StorageError> {
        Ok(sqlx::query_as(
            "insert into onchain_addresses (id, project_id, address, label, required_confirmations)
             values ($1, $2, $3, $4, $5) returning *",
        )
        .bind(AddressId::new().as_str())
        .bind(project_id)
        .bind(address)
        .bind(label)
        .bind(required_confirmations)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_address(
        &self,
        project_id: &str,
        id: &str,
    ) -> Result<AddressRecord, StorageError> {
        Ok(
            sqlx::query_as("select * from onchain_addresses where project_id = $1 and id = $2")
                .bind(project_id)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn watched_addresses(&self) -> Result<Vec<AddressRecord>, StorageError> {
        Ok(
            sqlx::query_as("select * from onchain_addresses order by created_at")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// Records what the chain watcher saw for one output. Returns the stored
    /// row when it is new or its confirmation count changed, `None` when
    /// nothing changed, so repeated scans produce no duplicate events.
    pub async fn record_observation(
        &self,
        obs: Observation<'_>,
    ) -> Result<Option<OnchainTransactionRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let existing: Option<OnchainTransactionRecord> = sqlx::query_as(
            "select * from onchain_transactions where txid = $1 and vout = $2 for update",
        )
        .bind(obs.txid)
        .bind(obs.vout)
        .fetch_optional(&mut *tx)
        .await?;
        let required = obs.address.required_confirmations.max(1) as u32;
        let status = OnchainStatus::from_confirmations(obs.confirmations.max(0) as u32, required);
        let (row, kind) = match existing {
            None => {
                let row: OnchainTransactionRecord = sqlx::query_as(
                    "insert into onchain_transactions
                       (id, project_id, address_id, txid, vout, amount_sat, block_height,
                        confirmations, status)
                     values ($1, $2, $3, $4, $5, $6, $7, $8, $9) returning *",
                )
                .bind(TransactionId::new().as_str())
                .bind(&obs.address.project_id)
                .bind(&obs.address.id)
                .bind(obs.txid)
                .bind(obs.vout)
                .bind(to_i64(obs.amount_sat)?)
                .bind(obs.block_height)
                .bind(obs.confirmations)
                .bind(status.as_str())
                .fetch_one(&mut *tx)
                .await?;
                (row, "bitcoin.transaction.detected")
            }
            Some(old) if old.confirmations == obs.confirmations => return Ok(None),
            Some(old) => {
                let row: OnchainTransactionRecord = sqlx::query_as(
                    "update onchain_transactions set confirmations = $2, block_height = $3,
                            status = $4, updated_at = now()
                     where id = $1 returning *",
                )
                .bind(&old.id)
                .bind(obs.confirmations)
                .bind(obs.block_height)
                .bind(status.as_str())
                .fetch_one(&mut *tx)
                .await?;
                let kind = if status == OnchainStatus::Confirmed && old.status != "confirmed" {
                    "bitcoin.transaction.confirmed"
                } else {
                    "bitcoin.transaction.updated"
                };
                (row, kind)
            }
        };
        emit(
            &mut tx,
            &row.project_id,
            kind,
            &row.id,
            transaction_event_data(&row, &obs.address.address),
            false,
        )
        .await?;
        tx.commit().await?;
        Ok(Some(row))
    }

    pub async fn get_onchain_transaction(
        &self,
        project_id: &str,
        id: &str,
    ) -> Result<OnchainTransactionRecord, StorageError> {
        Ok(
            sqlx::query_as("select * from onchain_transactions where project_id = $1 and id = $2")
                .bind(project_id)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn list_onchain_transactions(
        &self,
        project_id: &str,
        limit: i64,
    ) -> Result<Vec<OnchainTransactionRecord>, StorageError> {
        Ok(sqlx::query_as(
            "select * from onchain_transactions where project_id = $1
             order by first_seen_at desc, id desc limit $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn address_for_transaction(&self, address_id: &str) -> Result<String, StorageError> {
        let (address,): (String,) =
            sqlx::query_as("select address from onchain_addresses where id = $1")
                .bind(address_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(address)
    }
}
