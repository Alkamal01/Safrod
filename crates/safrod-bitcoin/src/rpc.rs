use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RpcError {
    #[error("bitcoin rpc transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("bitcoin rpc error {code}: {message}")]
    Remote { code: i64, message: String },
    #[error("unexpected bitcoin rpc response: {0}")]
    Malformed(String),
}

#[derive(Clone)]
pub struct BitcoinRpc {
    http: reqwest::Client,
    url: String,
    user: String,
    password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unspent {
    pub txid: String,
    pub vout: u32,
    pub address: String,
    pub amount_sat: u64,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolOutput {
    pub txid: String,
    pub vout: u32,
    pub address: String,
    pub amount_sat: u64,
}

#[derive(Debug)]
pub struct ScanResult {
    pub tip_height: u32,
    pub unspents: Vec<Unspent>,
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    result: Option<&'a RawValue>,
    error: Option<RemoteError>,
}

#[derive(Deserialize)]
struct RemoteError {
    code: i64,
    message: String,
}

impl BitcoinRpc {
    pub fn new(url: &str, user: &str, password: &str) -> Result<Self, RpcError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            http,
            url: url.trim_end_matches('/').to_string(),
            user: user.to_string(),
            password: password.to_string(),
        })
    }

    /// Returns the raw `result` text so amounts keep their exact decimal form
    /// instead of passing through `f64`.
    async fn call_raw(&self, method: &str, params: Value) -> Result<String, RpcError> {
        let body = json!({ "jsonrpc": "1.0", "id": "safrod", "method": method, "params": params });
        let text = self
            .http
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.password))
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        let envelope: Envelope =
            serde_json::from_str(&text).map_err(|_| RpcError::Malformed(truncate(&text)))?;
        if let Some(err) = envelope.error {
            return Err(RpcError::Remote {
                code: err.code,
                message: err.message,
            });
        }
        Ok(envelope
            .result
            .map(|r| r.get().to_string())
            .unwrap_or_else(|| "null".to_string()))
    }

    pub async fn chain(&self) -> Result<String, RpcError> {
        let raw = self.call_raw("getblockchaininfo", json!([])).await?;
        let v: Value =
            serde_json::from_str(&raw).map_err(|_| RpcError::Malformed(truncate(&raw)))?;
        v["chain"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| RpcError::Malformed(truncate(&raw)))
    }

    pub async fn block_count(&self) -> Result<u32, RpcError> {
        let raw = self.call_raw("getblockcount", json!([])).await?;
        raw.trim()
            .parse()
            .map_err(|_| RpcError::Malformed(truncate(&raw)))
    }

    pub async fn mempool_txids(&self) -> Result<Vec<String>, RpcError> {
        let raw = self.call_raw("getrawmempool", json!([false])).await?;
        serde_json::from_str(&raw).map_err(|_| RpcError::Malformed(truncate(&raw)))
    }

    /// Outputs of a mempool transaction. A transaction that confirmed or was
    /// evicted between listing and fetching yields `None`.
    pub async fn mempool_outputs(
        &self,
        txid: &str,
    ) -> Result<Option<Vec<MempoolOutput>>, RpcError> {
        let raw = match self
            .call_raw("getrawtransaction", json!([txid, true]))
            .await
        {
            Ok(raw) => raw,
            Err(RpcError::Remote { code: -5, .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let tx: RawTx =
            serde_json::from_str(&raw).map_err(|_| RpcError::Malformed(truncate(&raw)))?;
        if tx.confirmations.unwrap_or(0) > 0 {
            return Ok(None);
        }
        let mut outputs = Vec::new();
        for vout in tx.vout {
            let Some(address) = vout.script_pub_key.address else {
                continue;
            };
            outputs.push(MempoolOutput {
                txid: txid.to_string(),
                vout: vout.n,
                address,
                amount_sat: btc_to_sats(vout.value.get())?,
            });
        }
        Ok(Some(outputs))
    }

    /// Scans the confirmed UTXO set for the given addresses in one call.
    /// Bitcoin Core only allows one scan at a time, so callers must not run
    /// this concurrently.
    pub async fn scan_addresses(&self, addresses: &[String]) -> Result<ScanResult, RpcError> {
        let objects: Vec<String> = addresses.iter().map(|a| format!("addr({a})")).collect();
        let raw = self
            .call_raw("scantxoutset", json!(["start", objects]))
            .await?;
        let scan: RawScan =
            serde_json::from_str(&raw).map_err(|_| RpcError::Malformed(truncate(&raw)))?;
        let mut unspents = Vec::new();
        for u in scan.unspents {
            let address = address_for_script(addresses, &u.desc)
                .ok_or_else(|| RpcError::Malformed(format!("unmatched descriptor {}", u.desc)))?;
            unspents.push(Unspent {
                txid: u.txid,
                vout: u.vout,
                address,
                amount_sat: btc_to_sats(u.amount.get())?,
                height: u.height,
            });
        }
        Ok(ScanResult {
            tip_height: scan.height,
            unspents,
        })
    }

    pub async fn generate_to_address(
        &self,
        count: u32,
        address: &str,
    ) -> Result<Vec<String>, RpcError> {
        let raw = self
            .call_raw("generatetoaddress", json!([count, address]))
            .await?;
        serde_json::from_str(&raw).map_err(|_| RpcError::Malformed(truncate(&raw)))
    }
}

#[derive(Deserialize)]
struct RawTx<'a> {
    confirmations: Option<u32>,
    #[serde(borrow)]
    vout: Vec<RawVout<'a>>,
}

#[derive(Deserialize)]
struct RawVout<'a> {
    #[serde(borrow)]
    value: &'a RawValue,
    n: u32,
    #[serde(rename = "scriptPubKey")]
    script_pub_key: RawScript,
}

#[derive(Deserialize)]
struct RawScript {
    address: Option<String>,
}

#[derive(Deserialize)]
struct RawScan<'a> {
    height: u32,
    #[serde(borrow)]
    unspents: Vec<RawUnspent<'a>>,
}

#[derive(Deserialize)]
struct RawUnspent<'a> {
    txid: String,
    vout: u32,
    desc: String,
    #[serde(borrow)]
    amount: &'a RawValue,
    height: u32,
}

/// `scantxoutset` reports each hit with the matching descriptor
/// (`addr(bcrt1…)#checksum`), which identifies the watched address.
fn address_for_script(addresses: &[String], desc: &str) -> Option<String> {
    addresses
        .iter()
        .find(|a| desc.starts_with(&format!("addr({a})")))
        .cloned()
}

/// Converts Bitcoin Core's fixed-point BTC string (`0.00025000`) to satoshis
/// without going through floating point.
pub fn btc_to_sats(text: &str) -> Result<u64, RpcError> {
    let bad = || RpcError::Malformed(format!("bad amount {text}"));
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || frac.len() > 8
        || !whole
            .bytes()
            .chain(frac.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(bad());
    }
    let whole: u64 = whole.parse().map_err(|_| bad())?;
    let frac: u64 = format!("{frac:0<8}").parse().map_err(|_| bad())?;
    whole
        .checked_mul(100_000_000)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(bad)
}

fn truncate(text: &str) -> String {
    text.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_amounts_exactly() {
        assert_eq!(btc_to_sats("0.00025000").unwrap(), 25_000);
        assert_eq!(btc_to_sats("1").unwrap(), 100_000_000);
        assert_eq!(btc_to_sats("12.5").unwrap(), 1_250_000_000);
        assert_eq!(btc_to_sats("0.00000001").unwrap(), 1);
    }

    #[test]
    fn rejects_malformed_amounts() {
        assert!(btc_to_sats("1e-5").is_err());
        assert!(btc_to_sats("-1").is_err());
        assert!(btc_to_sats("0.123456789").is_err());
        assert!(btc_to_sats("").is_err());
        assert!(btc_to_sats("99999999999999999999").is_err());
    }

    #[test]
    fn descriptor_maps_back_to_address() {
        let addrs = vec!["bcrt1qaaa".to_string(), "bcrt1qbbb".to_string()];
        assert_eq!(
            address_for_script(&addrs, "addr(bcrt1qbbb)#abcd1234"),
            Some("bcrt1qbbb".to_string())
        );
        assert_eq!(address_for_script(&addrs, "addr(other)#x"), None);
    }
}
