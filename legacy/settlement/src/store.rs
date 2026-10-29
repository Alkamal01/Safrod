use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::attestation::PayoutAttestation;
use crate::ids::{ReservationId, SettlementId};
use crate::lightning::Bolt11Invoice;
use crate::money::{FiatAmount, Millisatoshis};
use crate::quote::ExecutableQuote;
use crate::state::SettlementState;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SettlementRecord {
    pub id: SettlementId,
    pub state: SettlementState,
    pub fiat_amount: FiatAmount,
    pub btc_amount: Millisatoshis,
    pub counterparty: String,
    pub beneficiary: String,
    pub payout_idempotency_key: String,
    pub quote_expires_at_unix: Option<u64>,
    pub quote: Option<ExecutableQuote>,
    pub source_reservation: Option<ReservationId>,
    pub counterparty_invoice: Option<Bolt11Invoice>,
    pub attestations: Vec<PayoutAttestation>,
}

/// A small crash-safe JSON journal for the reference daemon. A single lock
/// file prevents two processes from writing the same node state.
pub struct FileSettlementStore {
    path: PathBuf,
    _lock: File,
    records: Mutex<HashMap<SettlementId, SettlementRecord>>,
}

impl FileSettlementStore {
    pub fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        lock.try_lock()?;
        let records = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            _lock: lock,
            records: Mutex::new(records),
        })
    }

    fn save(&self, records: &HashMap<SettlementId, SettlementRecord>) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(records).map_err(std::io::Error::other)?;
        let temporary = self.path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(temporary, &self.path)?;
        if let Some(parent) = self.path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

pub trait SettlementStore: Send + Sync {
    fn put(&self, record: SettlementRecord);
    fn get(&self, id: SettlementId) -> Option<SettlementRecord>;
    fn list_pending(&self) -> Vec<SettlementRecord>;
}

pub struct InMemorySettlementStore {
    records: Mutex<HashMap<SettlementId, SettlementRecord>>,
}
impl InMemorySettlementStore {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
        }
    }
}
impl Default for InMemorySettlementStore {
    fn default() -> Self {
        Self::new()
    }
}
impl SettlementStore for InMemorySettlementStore {
    fn put(&self, record: SettlementRecord) {
        self.records
            .lock()
            .expect("settlement store mutex poisoned")
            .insert(record.id, record);
    }
    fn get(&self, id: SettlementId) -> Option<SettlementRecord> {
        self.records
            .lock()
            .expect("settlement store mutex poisoned")
            .get(&id)
            .cloned()
    }
    fn list_pending(&self) -> Vec<SettlementRecord> {
        self.records
            .lock()
            .expect("settlement store mutex poisoned")
            .values()
            .filter(|record| {
                !matches!(
                    record.state,
                    SettlementState::SettlementReleased | SettlementState::Refunded
                )
            })
            .cloned()
            .collect()
    }
}

impl SettlementStore for FileSettlementStore {
    fn put(&self, record: SettlementRecord) {
        let mut records = self
            .records
            .lock()
            .expect("settlement store mutex poisoned");
        let id = record.id;
        let old = records.insert(id, record);
        if let Err(error) = self.save(&records) {
            match old {
                Some(record) => {
                    records.insert(record.id, record);
                }
                None => {
                    records.remove(&id);
                }
            }
            panic!("failed to persist settlement state: {error}");
        }
    }
    fn get(&self, id: SettlementId) -> Option<SettlementRecord> {
        self.records
            .lock()
            .expect("settlement store mutex poisoned")
            .get(&id)
            .cloned()
    }
    fn list_pending(&self) -> Vec<SettlementRecord> {
        self.records
            .lock()
            .expect("settlement store mutex poisoned")
            .values()
            .filter(|record| {
                !matches!(
                    record.state,
                    SettlementState::SettlementReleased | SettlementState::Refunded
                )
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::Currency;

    fn record(id: SettlementId) -> SettlementRecord {
        SettlementRecord {
            id,
            state: SettlementState::SettlementConditionCreated,
            fiat_amount: FiatAmount::new(Currency::Ngn, 1_000),
            btc_amount: Millisatoshis(1_000),
            counterparty: "kes-node".into(),
            beneficiary: "254700000000".into(),
            payout_idempotency_key: id.to_string(),
            quote_expires_at_unix: None,
            quote: None,
            source_reservation: None,
            counterparty_invoice: Some(Bolt11Invoice("saved-invoice".into())),
            attestations: Vec::new(),
        }
    }

    #[test]
    fn file_store_survives_a_restart() {
        let path = std::env::temp_dir().join(format!("safro-store-{}.json", SettlementId::new()));
        let id = SettlementId::new();
        {
            let store = FileSettlementStore::open(&path).unwrap();
            store.put(record(id));
        }
        let reopened = FileSettlementStore::open(&path).unwrap();
        assert_eq!(
            reopened.get(id).unwrap().counterparty_invoice,
            Some(Bolt11Invoice("saved-invoice".into()))
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(path.with_extension("lock")).unwrap();
    }
}
