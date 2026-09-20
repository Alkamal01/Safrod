//! An in-memory `FiatAdapter` used for the v0.1 demo and tests.
use std::collections::HashMap;
use std::sync::Mutex;

use crate::adapter::{AdapterError, FiatAdapter};
use crate::ids::ReservationId;
use crate::money::FiatAmount;
use crate::state::PayoutOutcome;

#[derive(Debug, Clone)]
pub enum SimulatedPayoutBehavior {
    Confirmed,
    Rejected(String),
    ConfirmedAfterTimeout,
}

struct State {
    reservations: HashMap<ReservationId, FiatAmount>,
    payouts: HashMap<String, PayoutOutcome>,
    behaviors: HashMap<String, SimulatedPayoutBehavior>,
}
pub struct SimulatedFiatAdapter {
    state: Mutex<State>,
}
impl SimulatedFiatAdapter {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                reservations: HashMap::new(),
                payouts: HashMap::new(),
                behaviors: HashMap::new(),
            }),
        }
    }
    pub fn set_behavior(&self, key: impl Into<String>, behavior: SimulatedPayoutBehavior) {
        self.state
            .lock()
            .expect("simulated adapter mutex poisoned")
            .behaviors
            .insert(key.into(), behavior);
    }
}
impl Default for SimulatedFiatAdapter {
    fn default() -> Self {
        Self::new()
    }
}
impl FiatAdapter for SimulatedFiatAdapter {
    fn reserve(&self, amount: FiatAmount) -> Result<ReservationId, AdapterError> {
        let id = ReservationId::new();
        self.state
            .lock()
            .expect("simulated adapter mutex poisoned")
            .reservations
            .insert(id, amount);
        Ok(id)
    }
    fn initiate_payout(
        &self,
        reservation: ReservationId,
        _beneficiary: &str,
        key: &str,
    ) -> Result<PayoutOutcome, AdapterError> {
        let mut state = self.state.lock().expect("simulated adapter mutex poisoned");
        if !state.reservations.contains_key(&reservation) {
            return Err(AdapterError::ReservationNotFound);
        }
        if let Some(outcome) = state.payouts.get(key) {
            return Ok(*outcome);
        }
        match state
            .behaviors
            .get(key)
            .cloned()
            .unwrap_or(SimulatedPayoutBehavior::Confirmed)
        {
            SimulatedPayoutBehavior::Confirmed => {
                state.payouts.insert(key.into(), PayoutOutcome::Confirmed);
                Ok(PayoutOutcome::Confirmed)
            }
            SimulatedPayoutBehavior::Rejected(reason) => {
                state.payouts.insert(key.into(), PayoutOutcome::Failed);
                Err(AdapterError::Rejected(reason))
            }
            SimulatedPayoutBehavior::ConfirmedAfterTimeout => {
                state.payouts.insert(key.into(), PayoutOutcome::Confirmed);
                Err(AdapterError::Timeout)
            }
        }
    }
    fn query_payout(&self, key: &str) -> Result<Option<PayoutOutcome>, AdapterError> {
        Ok(self
            .state
            .lock()
            .expect("simulated adapter mutex poisoned")
            .payouts
            .get(key)
            .copied())
    }
    fn cancel_reservation(&self, reservation: ReservationId) -> Result<(), AdapterError> {
        self.state
            .lock()
            .expect("simulated adapter mutex poisoned")
            .reservations
            .remove(&reservation)
            .map(|_| ())
            .ok_or(AdapterError::ReservationNotFound)
    }
}
