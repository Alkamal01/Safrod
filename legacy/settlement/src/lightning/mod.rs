//! The boundary between Safro and the Lightning settlement layer.
//!
//! A Lightning payment is asymmetric: one provider's node is the payee
//! (the destination provider, who fronts the fiat payout and is
//! reimbursed over Lightning) and the other is the payer (the source
//! provider). "Conditional" settlement for v0.1 is enforced at the Safro
//! level, not by holding an in-flight HTLC: the payer obtains the
//! counterparty's invoice up front but does not pay it until the
//! settlement is released. True HTLC-level holding (accepting a payment
//! but deferring the fulfill/fail decision inside Lightning itself) needs
//! a custom `htlc_accepted`-hook plugin and carries its own operational
//! risk (a held HTLC risks the channel being force-closed if held too
//! long) — that's real additional scope, tracked as a follow-up rather
//! than built into v0.1.
//!
//! [`cln`] is the only implementation: a real Core Lightning regtest node
//! over its native JSON-RPC (Unix socket) interface.

pub mod cln;

use crate::ids::SettlementId;
use crate::money::Millisatoshis;

#[derive(Debug, Clone, thiserror::Error)]
pub enum LightningError {
    #[error("insufficient channel liquidity for this settlement")]
    InsufficientLiquidity,
    #[error("the counterparty is unreachable")]
    PeerUnreachable,
    #[error("invoice amount does not match the expected settlement amount")]
    AmountMismatch,
    #[error("invoice could not be parsed: {0}")]
    MalformedInvoice(String),
    #[error("no settlement invoice was recorded for this settlement id")]
    NoRecordedInvoice,
    #[error("lightning backend error: {0}")]
    Backend(String),
}

/// A BOLT11 invoice string, opaque to everything except the Lightning
/// backend that validates and pays it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bolt11Invoice(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentPreimage(pub [u8; 32]);

pub trait LightningSettlement: Send + Sync {
    /// Payee side: creates a real invoice for `amount_msat` tied to this
    /// settlement, to be handed to the counterparty over the peer API. No
    /// funds move yet.
    fn create_settlement_invoice(
        &self,
        settlement_id: SettlementId,
        amount: Millisatoshis,
    ) -> Result<Bolt11Invoice, LightningError>;

    /// Payer side: validates the counterparty's invoice amount against
    /// what this settlement expects and records it, without paying it —
    /// this is what makes the settlement conditional. Funds only move
    /// once `release` is called.
    fn accept_settlement_invoice(
        &self,
        settlement_id: SettlementId,
        expected_amount: Millisatoshis,
        invoice: Bolt11Invoice,
    ) -> Result<(), LightningError>;

    /// Payer side: pays the previously accepted invoice now.
    fn release(&self, settlement_id: SettlementId) -> Result<PaymentPreimage, LightningError>;

    /// Payer side: discards a previously accepted invoice without paying
    /// it. No Lightning-level action is needed since nothing was sent.
    fn refund(&self, settlement_id: SettlementId) -> Result<(), LightningError>;
}
