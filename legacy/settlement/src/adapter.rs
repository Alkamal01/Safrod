//! The boundary between Safro and an external fiat payment rail.
use crate::ids::ReservationId;
use crate::money::FiatAmount;
use crate::state::PayoutOutcome;

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("fiat rail rejected the request: {0}")]
    Rejected(String),
    #[error("no response was received before the request timed out")]
    Timeout,
    #[error("reservation not found")]
    ReservationNotFound,
}

pub trait FiatAdapter: Send + Sync {
    fn reserve(&self, amount: FiatAmount) -> Result<ReservationId, AdapterError>;
    fn initiate_payout(
        &self,
        reservation: ReservationId,
        beneficiary: &str,
        idempotency_key: &str,
    ) -> Result<PayoutOutcome, AdapterError>;
    fn query_payout(&self, idempotency_key: &str) -> Result<Option<PayoutOutcome>, AdapterError>;
    fn cancel_reservation(&self, reservation: ReservationId) -> Result<(), AdapterError>;
}
