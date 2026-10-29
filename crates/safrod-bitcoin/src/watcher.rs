use std::collections::HashMap;
use std::time::Duration;

use safrod_storage::Db;
use safrod_storage::models::AddressRecord;
use safrod_storage::onchain::Observation;
use tokio::sync::watch;
use tracing::{error, info};

use crate::rpc::{BitcoinRpc, RpcError};

const MAX_MEMPOOL_TXS: usize = 2_000;

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error(transparent)]
    Rpc(#[from] RpcError),
    #[error(transparent)]
    Storage(#[from] safrod_storage::StorageError),
}

pub struct Watcher {
    rpc: BitcoinRpc,
    db: Db,
}

impl Watcher {
    pub fn new(rpc: BitcoinRpc, db: Db) -> Self {
        Self { rpc, db }
    }

    /// One pass over the mempool and the confirmed UTXO set. Returns how many
    /// outputs changed. Observations are idempotent, so running this twice in
    /// a row emits nothing the second time.
    pub async fn scan_once(&self) -> Result<usize, WatchError> {
        let addresses = self.db.watched_addresses().await?;
        if addresses.is_empty() {
            return Ok(0);
        }
        let by_address: HashMap<&str, &AddressRecord> =
            addresses.iter().map(|a| (a.address.as_str(), a)).collect();
        let mut changed = 0;

        let mut seen_confirmed = HashMap::new();
        let names: Vec<String> = addresses.iter().map(|a| a.address.clone()).collect();
        let scan = self.rpc.scan_addresses(&names).await?;
        for u in &scan.unspents {
            let Some(address) = by_address.get(u.address.as_str()) else {
                continue;
            };
            let confirmations = (scan.tip_height + 1).saturating_sub(u.height) as i32;
            seen_confirmed.insert((u.txid.clone(), u.vout), ());
            let observed = self
                .db
                .record_observation(Observation {
                    address,
                    txid: &u.txid,
                    vout: u.vout as i32,
                    amount_sat: u.amount_sat,
                    block_height: Some(u.height as i32),
                    confirmations,
                })
                .await?;
            changed += usize::from(observed.is_some());
        }

        let txids = self.rpc.mempool_txids().await?;
        for txid in txids.iter().take(MAX_MEMPOOL_TXS) {
            let Some(outputs) = self.rpc.mempool_outputs(txid).await? else {
                continue;
            };
            for out in outputs {
                let Some(address) = by_address.get(out.address.as_str()) else {
                    continue;
                };
                if seen_confirmed.contains_key(&(out.txid.clone(), out.vout)) {
                    continue;
                }
                let observed = self
                    .db
                    .record_observation(Observation {
                        address,
                        txid: &out.txid,
                        vout: out.vout as i32,
                        amount_sat: out.amount_sat,
                        block_height: None,
                        confirmations: 0,
                    })
                    .await?;
                changed += usize::from(observed.is_some());
            }
        }
        Ok(changed)
    }

    pub async fn run(self, interval: Duration, mut shutdown: watch::Receiver<bool>) {
        info!(?interval, "on-chain watcher started");
        loop {
            match self.scan_once().await {
                Ok(0) => {}
                Ok(n) => info!(changed = n, "on-chain observations updated"),
                Err(e) => error!(error = %e, "on-chain scan failed"),
            }
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = shutdown.changed() => break,
            }
        }
    }
}
