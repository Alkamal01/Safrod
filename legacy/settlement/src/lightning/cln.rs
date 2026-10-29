//! A `LightningSettlement` backed by a real Core Lightning node, talked to
//! over its native JSON-RPC interface (a Unix domain socket at
//! `<lightning-dir>/<network>/lightning-rpc`).
//!
//! This is a minimal hand-rolled client rather than the generated
//! `cln-rpc` crate: the wire protocol (line-delimited JSON-RPC 2.0,
//! responses terminated by `\n\n`) is small, stable, and was verified
//! directly against a running `lightningd` rather than guessed at.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use lightning_invoice::Bolt11Invoice as ParsedInvoice;
use serde_json::{Value, json};

use super::{Bolt11Invoice, LightningError, LightningSettlement, PaymentPreimage};
use crate::ids::SettlementId;
use crate::money::Millisatoshis;

struct RpcClient {
    socket_path: PathBuf,
    next_id: AtomicU64,
}

impl RpcClient {
    fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            next_id: AtomicU64::new(1),
        }
    }

    /// Opens a fresh connection per call. Simpler and safer than
    /// multiplexing requests over one persistent socket, at the cost of a
    /// connection setup per RPC — fine for a local Unix socket at v0.1's
    /// call volume.
    fn call(&self, method: &str, params: Value) -> Result<Value, LightningError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut stream = UnixStream::connect(&self.socket_path)
            .map_err(|e| LightningError::Backend(format!("connecting to lightningd: {e}")))?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

        let mut payload = serde_json::to_vec(&request)
            .map_err(|e| LightningError::Backend(format!("encoding request: {e}")))?;
        payload.extend_from_slice(b"\n\n");
        stream
            .write_all(&payload)
            .map_err(|e| LightningError::Backend(format!("writing request: {e}")))?;

        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream
                .read(&mut chunk)
                .map_err(|e| LightningError::Backend(format!("reading response: {e}")))?;
            if n == 0 {
                return Err(LightningError::Backend(
                    "lightningd closed the connection".to_string(),
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(2).any(|w| w == b"\n\n") {
                break;
            }
        }

        let response: Value = serde_json::from_slice(&buf)
            .map_err(|e| LightningError::Backend(format!("parsing response: {e}")))?;

        if let Some(error) = response.get("error") {
            return Err(LightningError::Backend(format!(
                "lightningd error: {error}"
            )));
        }

        response.get("result").cloned().ok_or_else(|| {
            LightningError::Backend("response had neither result nor error".to_string())
        })
    }
}

fn hex_decode_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub struct ClnLightningSettlement {
    rpc: RpcClient,
    /// Invoices this node (as payer) has accepted for a settlement but not
    /// yet paid. Lost on restart — durability is the same flagged
    /// follow-up as [`crate::store::InMemorySettlementStore`].
    pending_invoices: Mutex<HashMap<SettlementId, Bolt11Invoice>>,
}

impl ClnLightningSettlement {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            rpc: RpcClient::new(socket_path),
            pending_invoices: Mutex::new(HashMap::new()),
        }
    }
}

impl LightningSettlement for ClnLightningSettlement {
    fn create_settlement_invoice(
        &self,
        settlement_id: SettlementId,
        amount: Millisatoshis,
    ) -> Result<Bolt11Invoice, LightningError> {
        let label = format!("safro-settlement-{settlement_id}");
        let result = self.rpc.call(
            "invoice",
            json!({
                "amount_msat": amount.0,
                "label": label,
                "description": format!("Safro settlement {settlement_id}"),
            }),
        )?;

        let bolt11 = result
            .get("bolt11")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                LightningError::Backend("invoice response missing bolt11".to_string())
            })?;

        Ok(Bolt11Invoice(bolt11.to_string()))
    }

    fn accept_settlement_invoice(
        &self,
        settlement_id: SettlementId,
        expected_amount: Millisatoshis,
        invoice: Bolt11Invoice,
    ) -> Result<(), LightningError> {
        let parsed: ParsedInvoice = invoice
            .0
            .parse()
            .map_err(|e| LightningError::MalformedInvoice(format!("{e:?}")))?;

        let invoice_amount_msat = parsed.amount_milli_satoshis().ok_or_else(|| {
            LightningError::MalformedInvoice("invoice has no fixed amount".to_string())
        })?;

        if invoice_amount_msat != expected_amount.0 {
            return Err(LightningError::AmountMismatch);
        }

        self.pending_invoices
            .lock()
            .expect("pending invoices mutex poisoned")
            .insert(settlement_id, invoice);
        Ok(())
    }

    fn release(&self, settlement_id: SettlementId) -> Result<PaymentPreimage, LightningError> {
        let invoice = self
            .pending_invoices
            .lock()
            .expect("pending invoices mutex poisoned")
            .get(&settlement_id)
            .cloned()
            .ok_or(LightningError::NoRecordedInvoice)?;

        // Keep the same invoice on errors, including a lost response after
        // payment. CLN's `pay` returns success for an already-paid invoice.
        let result = self.rpc.call("pay", json!({ "bolt11": invoice.0 }))?;

        let preimage_hex = result
            .get("payment_preimage")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                LightningError::Backend("pay response missing payment_preimage".to_string())
            })?;

        let preimage = hex_decode_32(preimage_hex).ok_or_else(|| {
            LightningError::Backend("payment_preimage was not 32 bytes of hex".to_string())
        })?;

        self.pending_invoices
            .lock()
            .expect("pending invoices mutex poisoned")
            .remove(&settlement_id);
        Ok(PaymentPreimage(preimage))
    }

    fn refund(&self, settlement_id: SettlementId) -> Result<(), LightningError> {
        self.pending_invoices
            .lock()
            .expect("pending invoices mutex poisoned")
            .remove(&settlement_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    struct TestSocket(PathBuf);

    impl Drop for TestSocket {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn assert_release_can_retry(first_response: Option<Value>) {
        let id = SettlementId::new();
        let socket = TestSocket(std::env::temp_dir().join(format!("safro-{id}.sock")));
        let listener = UnixListener::bind(&socket.0).unwrap();
        let backend = ClnLightningSettlement::new(socket.0.clone());
        // These tests exercise RPC recovery; invoice parsing is bypassed.
        backend
            .pending_invoices
            .lock()
            .unwrap()
            .insert(id, Bolt11Invoice("same-invoice-on-every-attempt".into()));
        let server = std::thread::spawn(move || {
            let success = json!({"result": {"payment_preimage": "07".repeat(32)}});
            for response in [first_response, Some(success)] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\n\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let request: Value = serde_json::from_slice(&request).unwrap();
                assert_eq!(request["method"], "pay");
                assert_eq!(request["params"]["bolt11"], "same-invoice-on-every-attempt");
                if let Some(mut response) = response {
                    response["id"] = request["id"].clone();
                    response["jsonrpc"] = json!("2.0");
                    let mut bytes = serde_json::to_vec(&response).unwrap();
                    bytes.extend_from_slice(b"\n\n");
                    stream.write_all(&bytes).unwrap();
                }
                // None models CLN accepting the request but losing its reply.
            }
        });

        assert!(matches!(
            backend.release(id),
            Err(LightningError::Backend(_))
        ));
        assert_eq!(backend.release(id).unwrap(), PaymentPreimage([7u8; 32]));
        assert!(matches!(
            backend.release(id),
            Err(LightningError::NoRecordedInvoice)
        ));
        server.join().unwrap();
    }

    #[test]
    fn failed_payment_can_retry_the_same_invoice() {
        assert_release_can_retry(Some(json!({"error": {"code": 205, "message": "no route"}})));
    }

    #[test]
    fn lost_payment_response_can_retry_the_same_invoice() {
        assert_release_can_retry(None);
    }

    #[test]
    fn malformed_payment_result_preserves_invoice_for_retry() {
        assert_release_can_retry(Some(json!({"result": {"payment_preimage": "invalid"}})));
    }
}
