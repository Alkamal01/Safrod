//! The Lightning surface Safrod needs, kept small enough that the payment
//! engine and API can be tested against `mock::MockBackend` while the real
//! implementation is `ldk::LdkBackend`.
//!
//! All methods are blocking. LDK Node runs its own runtime and some calls block
//! on it, so async callers must go through `tokio::task::spawn_blocking`.

pub mod ldk;
pub mod mock;

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LightningError {
    #[error("invalid invoice: {0}")]
    InvalidInvoice(String),
    #[error("invoice is for {found}, this node runs on {expected}")]
    NetworkMismatch {
        expected: &'static str,
        found: &'static str,
    },
    #[error("invoice has expired")]
    InvoiceExpired,
    #[error("invoice has no amount and none was supplied")]
    AmountRequired,
    #[error("this invoice is already being paid or was already paid")]
    DuplicatePayment,
    #[error("insufficient outbound liquidity or no route")]
    NoRoute,
    #[error("the node is not running")]
    NotRunning,
    #[error("node error: {0}")]
    Node(String),
}

#[derive(Debug, Clone)]
pub struct NewInvoice {
    pub amount_msat: u64,
    pub description: String,
    pub expiry_secs: u32,
}

#[derive(Debug, Clone)]
pub struct CreatedInvoice {
    pub bolt11: String,
    /// Hex-encoded 32-byte payment hash.
    pub payment_hash: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvoice {
    pub payment_hash: String,
    pub amount_msat: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodePaymentStatus {
    Pending,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
pub struct NodePayment {
    pub status: NodePaymentStatus,
    pub preimage: Option<String>,
    pub fee_msat: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeInfo {
    pub node_id: String,
    pub network: &'static str,
    pub listening_addresses: Vec<String>,
    pub best_block_height: u32,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelInfo {
    pub channel_id: String,
    pub peer_node_id: String,
    pub capacity_sat: u64,
    pub outbound_capacity_msat: u64,
    pub inbound_capacity_msat: u64,
    pub is_ready: bool,
    pub is_usable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeBalances {
    pub onchain_total_sat: u64,
    pub onchain_spendable_sat: u64,
    pub lightning_total_sat: u64,
    pub outbound_capacity_msat: u64,
    pub inbound_capacity_msat: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeEvent {
    PaymentReceived {
        payment_hash: String,
        amount_msat: u64,
    },
    PaymentSuccessful {
        payment_hash: String,
        preimage: Option<String>,
        fee_msat: Option<u64>,
    },
    PaymentFailed {
        payment_hash: Option<String>,
        reason: String,
    },
    ChannelReady {
        channel_id: String,
    },
    ChannelClosed {
        channel_id: String,
        reason: String,
    },
    Other(String),
}

pub trait LightningBackend: Send + Sync {
    fn info(&self) -> NodeInfo;
    fn create_invoice(&self, request: &NewInvoice) -> Result<CreatedInvoice, LightningError>;

    /// Validates an invoice without paying it. Rejects wrong network,
    /// expired, and malformed invoices.
    fn parse_invoice(&self, bolt11: &str) -> Result<ParsedInvoice, LightningError>;

    /// Hands the payment to the node and returns once it is in flight, which
    /// does not mean it succeeded. The outcome arrives as a `NodeEvent`.
    fn send_payment(
        &self,
        bolt11: &str,
        amount_msat: Option<u64>,
    ) -> Result<String, LightningError>;

    /// The node's own record of a payment, used to reconcile after a restart.
    fn payment(&self, payment_hash: &str) -> Option<NodePayment>;

    fn new_onchain_address(&self) -> Result<String, LightningError>;
    fn channels(&self) -> Vec<ChannelInfo>;
    fn balances(&self) -> NodeBalances;
}
