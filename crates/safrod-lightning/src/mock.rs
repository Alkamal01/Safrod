//! In-memory backend for tests of the engine and API. It never moves funds and
//! is not used by the daemon unless explicitly selected for development.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::{
    ChannelInfo, CreatedInvoice, LightningBackend, LightningError, NewInvoice, NodeBalances,
    NodeInfo, NodePayment, ParsedInvoice,
};

#[derive(Default)]
pub struct MockBackend {
    state: Mutex<MockState>,
}

#[derive(Default)]
struct MockState {
    counter: u64,
    payments: HashMap<String, NodePayment>,
    fail_next_send: Option<LightningError>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fail_next_send(&self, err: LightningError) {
        self.state.lock().unwrap().fail_next_send = Some(err);
    }

    pub fn set_payment(&self, payment_hash: &str, payment: NodePayment) {
        self.state
            .lock()
            .unwrap()
            .payments
            .insert(payment_hash.to_string(), payment);
    }
}

fn fake_hash(counter: u64) -> String {
    format!("{counter:064x}")
}

impl LightningBackend for MockBackend {
    fn info(&self) -> NodeInfo {
        NodeInfo {
            node_id: "02".to_string() + &"ab".repeat(32),
            network: "regtest",
            listening_addresses: vec!["127.0.0.1:0".into()],
            best_block_height: 101,
            running: true,
        }
    }

    fn create_invoice(&self, request: &NewInvoice) -> Result<CreatedInvoice, LightningError> {
        let mut state = self.state.lock().unwrap();
        state.counter += 1;
        let hash = fake_hash(state.counter);
        Ok(CreatedInvoice {
            bolt11: format!("lnbcrt{}n1mock{hash}", request.amount_msat / 100),
            payment_hash: hash,
            expires_at_unix: 4_000_000_000,
        })
    }

    fn parse_invoice(&self, bolt11: &str) -> Result<ParsedInvoice, LightningError> {
        let hash = bolt11
            .split("mock")
            .nth(1)
            .filter(|h| h.len() == 64)
            .ok_or_else(|| LightningError::InvalidInvoice("not a mock invoice".into()))?;
        safrod_core::network::Network::Regtest
            .check_invoice_prefix(bolt11)
            .map_err(|e| LightningError::InvalidInvoice(e.to_string()))?;
        Ok(ParsedInvoice {
            payment_hash: hash.to_string(),
            amount_msat: Some(10_000),
        })
    }

    fn send_payment(
        &self,
        bolt11: &str,
        _amount_msat: Option<u64>,
    ) -> Result<String, LightningError> {
        let parsed = self.parse_invoice(bolt11)?;
        let mut state = self.state.lock().unwrap();
        if let Some(err) = state.fail_next_send.take() {
            return Err(err);
        }
        if state.payments.contains_key(&parsed.payment_hash) {
            return Err(LightningError::DuplicatePayment);
        }
        state.payments.insert(
            parsed.payment_hash.clone(),
            NodePayment {
                status: crate::NodePaymentStatus::Pending,
                preimage: None,
                fee_msat: None,
            },
        );
        Ok(parsed.payment_hash)
    }

    fn payment(&self, payment_hash: &str) -> Option<NodePayment> {
        self.state
            .lock()
            .unwrap()
            .payments
            .get(payment_hash)
            .cloned()
    }

    fn new_onchain_address(&self) -> Result<String, LightningError> {
        let mut state = self.state.lock().unwrap();
        state.counter += 1;
        Ok(format!("bcrt1qmock{:034}", state.counter))
    }

    fn channels(&self) -> Vec<ChannelInfo> {
        Vec::new()
    }

    fn balances(&self) -> NodeBalances {
        NodeBalances {
            onchain_total_sat: 0,
            onchain_spendable_sat: 0,
            lightning_total_sat: 0,
            outbound_capacity_msat: 0,
            inbound_capacity_msat: 0,
        }
    }
}
