//! The settlement state machine (proposal section 6). Transitions are
//! driven by a single, exhaustively-matched function: an invalid transition
//! is always a typed error, never a silent state change.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementState {
    Created,
    Quoted,
    SourceReserved,
    DestinationReserved,
    SettlementConditionCreated,
    PayoutRequested,
    PayoutConfirmed,
    PayoutFailed,
    /// A real protocol state, not an error-handling shortcut: the fiat rail
    /// did not give a definite answer (e.g. the payout API call timed out).
    /// It must be reconciled explicitly, never silently retried or treated
    /// as failure.
    PayoutUnknown,
    SettlementReleased,
    Refunded,
    Disputed,
}

/// The definite outcome of a fiat payout query, as reported by a
/// [`crate::adapter::FiatAdapter`] or reconciled from an attestation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum PayoutOutcome {
    Confirmed = 0,
    Failed = 1,
}

/// Inputs that drive a settlement's state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementEvent {
    Quote,
    ReserveSource,
    ReserveDestination,
    CreateSettlementCondition,
    RequestPayout,
    /// The fiat rail returned a definite `Confirmed`/`Failed`, or explicitly
    /// could not say (`None`), which becomes `PayoutUnknown`.
    ObservePayout(Option<PayoutOutcome>),
    /// Moves a `PayoutUnknown` settlement to a definite outcome once
    /// evidence has been reconciled (e.g. by querying the rail again).
    ReconcileUnknown(PayoutOutcome),
    ReleaseSettlement,
    Refund,
    /// Conflicting payout evidence was observed (e.g. two attestations that
    /// disagree) and the settlement needs manual resolution.
    Dispute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cannot apply {event:?} to settlement in state {from:?}")]
pub struct TransitionError {
    pub from: SettlementState,
    pub event: SettlementEvent,
}

pub fn transition(
    from: SettlementState,
    event: SettlementEvent,
) -> Result<SettlementState, TransitionError> {
    use PayoutOutcome::*;
    use SettlementEvent as E;
    use SettlementState as S;

    let to = match (from, event) {
        (S::Created, E::Quote) => S::Quoted,
        (S::Quoted, E::ReserveSource) => S::SourceReserved,
        (S::SourceReserved, E::ReserveDestination) => S::DestinationReserved,
        (S::DestinationReserved, E::CreateSettlementCondition) => S::SettlementConditionCreated,
        (S::SettlementConditionCreated, E::RequestPayout) => S::PayoutRequested,

        (S::PayoutRequested, E::ObservePayout(Some(Confirmed))) => S::PayoutConfirmed,
        (S::PayoutRequested, E::ObservePayout(Some(Failed))) => S::PayoutFailed,
        (S::PayoutRequested, E::ObservePayout(None)) => S::PayoutUnknown,

        (S::PayoutUnknown, E::ReconcileUnknown(Confirmed)) => S::PayoutConfirmed,
        (S::PayoutUnknown, E::ReconcileUnknown(Failed)) => S::PayoutFailed,

        (S::PayoutConfirmed, E::ReleaseSettlement) => S::SettlementReleased,
        (S::PayoutFailed, E::Refund) => S::Refunded,

        (
            S::PayoutRequested | S::PayoutUnknown | S::PayoutConfirmed | S::PayoutFailed,
            E::Dispute,
        ) => S::Disputed,

        (from, event) => return Err(TransitionError { from, event }),
    };

    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use PayoutOutcome::*;
    use SettlementEvent as E;
    use SettlementState as S;

    fn drive(events: &[E]) -> Result<S, TransitionError> {
        events
            .iter()
            .try_fold(S::Created, |state, &event| transition(state, event))
    }

    #[test]
    fn successful_settlement_happy_path() {
        let result = drive(&[
            E::Quote,
            E::ReserveSource,
            E::ReserveDestination,
            E::CreateSettlementCondition,
            E::RequestPayout,
            E::ObservePayout(Some(Confirmed)),
            E::ReleaseSettlement,
        ]);
        assert_eq!(result, Ok(S::SettlementReleased));
    }

    #[test]
    fn fiat_payout_rejected_leads_to_refund() {
        let result = drive(&[
            E::Quote,
            E::ReserveSource,
            E::ReserveDestination,
            E::CreateSettlementCondition,
            E::RequestPayout,
            E::ObservePayout(Some(Failed)),
            E::Refund,
        ]);
        assert_eq!(result, Ok(S::Refunded));
    }

    #[test]
    fn payout_timeout_does_not_mark_transaction_failed() {
        let after_timeout = drive(&[
            E::Quote,
            E::ReserveSource,
            E::ReserveDestination,
            E::CreateSettlementCondition,
            E::RequestPayout,
            E::ObservePayout(None),
        ]);
        assert_eq!(after_timeout, Ok(S::PayoutUnknown));

        // Reconciling to Failed from Unknown is a distinct, explicit step —
        // never inferred automatically from the timeout itself.
        let reconciled = transition(S::PayoutUnknown, E::ReconcileUnknown(Confirmed));
        assert_eq!(reconciled, Ok(S::PayoutConfirmed));
    }

    #[test]
    fn invalid_transition_is_rejected_not_silently_applied() {
        let result = transition(S::Created, E::RequestPayout);
        assert_eq!(
            result,
            Err(TransitionError {
                from: S::Created,
                event: E::RequestPayout
            })
        );
    }

    #[test]
    fn conflicting_payout_evidence_moves_to_disputed() {
        let result = transition(S::PayoutUnknown, E::Dispute);
        assert_eq!(result, Ok(S::Disputed));
    }
}
