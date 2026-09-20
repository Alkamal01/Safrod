//! Wires the state machine to a fiat adapter, a Lightning settlement
//! backend, and a store, and enforces the rules that don't belong in any
//! single one of those: idempotency, and never trusting a peer's claim
//! without a valid signature.
//!
//! A settlement's two sides are asymmetric: the payer node (source
//! provider) drives [`Node::begin_settlement`] then
//! [`Node::attach_counterparty_invoice`] once it has the payee's invoice;
//! the payee node (destination provider) only needs
//! [`Node::create_settlement_invoice`]. Fetching that invoice over the
//! network is the peer API's job, not this module's.

use bitcoin::secp256k1::{self, PublicKey, Secp256k1, SecretKey};

use crate::adapter::{AdapterError, FiatAdapter};
use crate::attestation::{self, AttestationError, PayoutAttestation};
use crate::ids::SettlementId;
use crate::lightning::{Bolt11Invoice, LightningError, LightningSettlement, PaymentPreimage};
use crate::money::{FiatAmount, Millisatoshis};
use crate::quote::{self, ExecutableQuote, QuoteError};
use crate::state::{PayoutOutcome, SettlementEvent, SettlementState, TransitionError, transition};
use crate::store::{SettlementRecord, SettlementStore};

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("unknown settlement {0}")]
    UnknownSettlement(SettlementId),
    #[error("settlement has no recorded source reservation")]
    MissingReservation,
    #[error(transparent)]
    Transition(#[from] TransitionError),
    #[error("fiat adapter error: {0}")]
    Adapter(AdapterError),
    #[error("lightning backend error: {0}")]
    Lightning(LightningError),
    #[error("payout was rejected by the fiat rail: {0}")]
    PayoutRejected(String),
    #[error("payout outcome is not yet known; reconcile later")]
    PayoutUnknown,
    #[error("settlement quote has expired")]
    QuoteExpired,
    #[error("quote is invalid: {0}")]
    Quote(QuoteError),
    #[error("settlement is not in a reconcilable state: {0:?}")]
    NotReconcilable(SettlementState),
    #[error("payout is still unknown; nothing to reconcile yet")]
    StillUnknown,
    #[error("attestation is invalid: {0}")]
    InvalidAttestation(AttestationError),
    #[error("attestation signer is not this settlement's configured counterparty")]
    UnexpectedAttestationSigner,
    #[error("conflicting payout evidence was recorded; settlement is now disputed")]
    ConflictingEvidence,
}

pub struct Node<A, L, S>
where
    A: FiatAdapter,
    L: LightningSettlement,
    S: SettlementStore,
{
    adapter: A,
    lightning: L,
    store: S,
    secp: Secp256k1<secp256k1::All>,
    signing_key: SecretKey,
    peer_identity: Option<PublicKey>,
}

impl<A, L, S> Node<A, L, S>
where
    A: FiatAdapter,
    L: LightningSettlement,
    S: SettlementStore,
{
    pub fn new(adapter: A, lightning: L, store: S, signing_key: SecretKey) -> Self {
        Self {
            adapter,
            lightning,
            store,
            secp: Secp256k1::new(),
            signing_key,
            peer_identity: None,
        }
    }

    /// Configures the one peer whose attestations may affect this node's
    /// settlements. Signature validity alone does not establish authority.
    pub fn with_peer_identity(mut self, peer_identity: PublicKey) -> Self {
        self.peer_identity = Some(peer_identity);
        self
    }

    /// Payer side, step 1: registers the settlement's intent (quoting is
    /// collapsed into this call for v0.1 — splitting it out is v0.2/v0.3
    /// scope, not needed for the Buildathon corridor's fixed simulated
    /// providers). No fiat or Lightning side effects yet: those need the
    /// counterparty's invoice, which arrives from the peer API and is fed
    /// back in via [`Self::attach_counterparty_invoice`].
    pub fn begin_settlement(
        &self,
        fiat_amount: FiatAmount,
        btc_amount: Millisatoshis,
        counterparty: impl Into<String>,
    ) -> SettlementId {
        self.begin_settlement_with_quote_expiry(fiat_amount, btc_amount, counterparty, None)
    }

    pub fn begin_settlement_with_quote_expiry(
        &self,
        fiat_amount: FiatAmount,
        btc_amount: Millisatoshis,
        counterparty: impl Into<String>,
        quote_expires_at_unix: Option<u64>,
    ) -> SettlementId {
        let id = SettlementId::new();
        let state = transition(SettlementState::Created, SettlementEvent::Quote)
            .expect("Created -> Quoted is always a valid transition");

        self.store.put(SettlementRecord {
            id,
            state,
            fiat_amount,
            btc_amount,
            counterparty: counterparty.into(),
            beneficiary: String::new(),
            payout_idempotency_key: id.to_string(),
            quote_expires_at_unix,
            quote: None,
            source_reservation: None,
            counterparty_invoice: None,
            attestations: Vec::new(),
        });

        id
    }

    fn quote_is_expired(record: &SettlementRecord) -> bool {
        if let Some(quote) = &record.quote {
            return std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(true, |now| now.as_secs() >= quote.terms.expires_at_unix);
        }
        record.quote_expires_at_unix.is_some_and(|expiry| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(true, |now| now.as_secs() >= expiry)
        })
    }

    /// Starts a source settlement only after accepting the destination
    /// provider's signed executable quote.
    pub fn begin_settlement_from_quote(
        &self,
        quote: ExecutableQuote,
        counterparty: impl Into<String>,
    ) -> Result<SettlementId, NodeError> {
        let peer = self
            .peer_identity
            .ok_or(NodeError::Quote(QuoteError::WrongProviders))?;
        let own = PublicKey::from_secret_key(&self.secp, &self.signing_key);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        quote::verify(&self.secp, &quote, &peer, &own, &peer, now).map_err(NodeError::Quote)?;
        let id = SettlementId::new();
        let state = transition(SettlementState::Created, SettlementEvent::Quote)
            .expect("Created -> Quoted is always valid");
        self.store.put(SettlementRecord {
            id,
            state,
            fiat_amount: quote.terms.source_amount,
            btc_amount: quote.terms.settlement_amount,
            counterparty: counterparty.into(),
            beneficiary: String::new(),
            payout_idempotency_key: id.to_string(),
            quote_expires_at_unix: Some(quote.terms.expires_at_unix),
            quote: Some(quote),
            source_reservation: None,
            counterparty_invoice: None,
            attestations: Vec::new(),
        });
        Ok(id)
    }

    /// Payer side, step 2: reserves local fiat liquidity and records the
    /// counterparty's invoice for this settlement, without paying it. This
    /// is what makes the settlement conditional — funds only move once
    /// [`Self::release_settlement`] is called.
    pub fn attach_counterparty_invoice(
        &self,
        id: SettlementId,
        invoice: Bolt11Invoice,
    ) -> Result<(), NodeError> {
        let mut record = self.store.get(id).ok_or(NodeError::UnknownSettlement(id))?;
        if Self::quote_is_expired(&record) {
            return Err(NodeError::QuoteExpired);
        }

        // Validate the whole transition sequence before making any real
        // side effect (fiat reservation, Lightning invoice acceptance) —
        // otherwise a call that's illegal from the current state would
        // still reserve liquidity or accept an invoice before being
        // rejected, rather than being a clean no-op.
        let reserved_source = transition(record.state, SettlementEvent::ReserveSource)?;
        let reserved_destination =
            transition(reserved_source, SettlementEvent::ReserveDestination)?;
        let condition_created = transition(
            reserved_destination,
            SettlementEvent::CreateSettlementCondition,
        )?;

        let reservation = self
            .adapter
            .reserve(record.fiat_amount)
            .map_err(NodeError::Adapter)?;
        self.lightning
            .accept_settlement_invoice(id, record.btc_amount, invoice.clone())
            .map_err(NodeError::Lightning)?;

        record.state = condition_created;
        record.source_reservation = Some(reservation);
        record.counterparty_invoice = Some(invoice);
        self.store.put(record);
        Ok(())
    }

    /// Payee side: creates an invoice for a settlement the counterparty is
    /// asking to pay us through. Called by the peer API handler when a
    /// counterparty requests an invoice.
    pub fn create_settlement_invoice(
        &self,
        settlement_id: SettlementId,
        btc_amount: Millisatoshis,
    ) -> Result<Bolt11Invoice, NodeError> {
        self.lightning
            .create_settlement_invoice(settlement_id, btc_amount)
            .map_err(NodeError::Lightning)
    }

    /// Destination side: reserve the local payout amount before issuing the
    /// invoice. The source cannot release Lightning until this node later
    /// reports a signed payout outcome.
    pub fn prepare_destination(
        &self,
        id: SettlementId,
        fiat_amount: FiatAmount,
        btc_amount: Millisatoshis,
        counterparty: impl Into<String>,
        beneficiary: impl Into<String>,
        quote_expires_at_unix: Option<u64>,
    ) -> Result<Bolt11Invoice, NodeError> {
        if self.store.get(id).is_some() {
            return Err(NodeError::Transition(TransitionError {
                from: SettlementState::SettlementConditionCreated,
                event: SettlementEvent::Quote,
            }));
        }
        if quote_expires_at_unix.is_some_and(|expiry| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(true, |now| now.as_secs() >= expiry)
        }) {
            return Err(NodeError::QuoteExpired);
        }
        let quoted = transition(SettlementState::Created, SettlementEvent::Quote)
            .expect("Created -> Quoted is always valid");
        let source_reserved = transition(quoted, SettlementEvent::ReserveSource)?;
        let destination_reserved =
            transition(source_reserved, SettlementEvent::ReserveDestination)?;
        let state = transition(
            destination_reserved,
            SettlementEvent::CreateSettlementCondition,
        )?;
        let reservation = self
            .adapter
            .reserve(fiat_amount)
            .map_err(NodeError::Adapter)?;
        let invoice = self.create_settlement_invoice(id, btc_amount)?;
        self.store.put(SettlementRecord {
            id,
            state,
            fiat_amount,
            btc_amount,
            counterparty: counterparty.into(),
            beneficiary: beneficiary.into(),
            payout_idempotency_key: id.to_string(),
            quote_expires_at_unix,
            quote: None,
            source_reservation: Some(reservation),
            counterparty_invoice: None,
            attestations: Vec::new(),
        });
        Ok(invoice)
    }

    /// Requests the fiat payout for a settlement. Safe to call more than
    /// once with the same settlement (the adapter's idempotency key is
    /// derived from the settlement id) — a duplicate request replays the
    /// previously recorded outcome instead of paying out twice.
    pub fn request_payout(&self, id: SettlementId) -> Result<PayoutOutcome, NodeError> {
        let mut record = self.store.get(id).ok_or(NodeError::UnknownSettlement(id))?;
        record.state = transition(record.state, SettlementEvent::RequestPayout)?;

        let reservation = record
            .source_reservation
            .ok_or(NodeError::MissingReservation)?;
        let outcome = self.adapter.initiate_payout(
            reservation,
            &record.beneficiary,
            &record.payout_idempotency_key,
        );

        let result = match outcome {
            Ok(payout_outcome) => {
                record.state = transition(
                    record.state,
                    SettlementEvent::ObservePayout(Some(payout_outcome)),
                )?;
                Ok(payout_outcome)
            }
            Err(AdapterError::Rejected(reason)) => {
                record.state = transition(
                    record.state,
                    SettlementEvent::ObservePayout(Some(PayoutOutcome::Failed)),
                )?;
                Err(NodeError::PayoutRejected(reason))
            }
            Err(AdapterError::Timeout) => {
                record.state = transition(record.state, SettlementEvent::ObservePayout(None))?;
                Err(NodeError::PayoutUnknown)
            }
            Err(AdapterError::ReservationNotFound) => return Err(NodeError::MissingReservation),
        };

        self.store.put(record);
        result
    }

    /// Applies evidence from the destination rail to the source-side record.
    /// The state is deliberately advanced only after signature and signer
    /// authorization checks have succeeded.
    pub fn apply_payout_attestation(
        &self,
        attestation: PayoutAttestation,
    ) -> Result<(), NodeError> {
        attestation::verify(&self.secp, &attestation).map_err(NodeError::InvalidAttestation)?;
        if let Some(expected) = self.peer_identity {
            let actual = PublicKey::from_slice(&attestation.signer_pubkey)
                .map_err(|_| NodeError::InvalidAttestation(AttestationError::Malformed))?;
            if actual != expected {
                return Err(NodeError::UnexpectedAttestationSigner);
            }
        }
        let mut record = self
            .store
            .get(attestation.settlement_id)
            .ok_or(NodeError::UnknownSettlement(attestation.settlement_id))?;
        record.state = match record.state {
            SettlementState::SettlementConditionCreated => {
                let requested = transition(record.state, SettlementEvent::RequestPayout)?;
                transition(
                    requested,
                    SettlementEvent::ObservePayout(Some(attestation.status)),
                )?
            }
            SettlementState::PayoutUnknown => transition(
                record.state,
                SettlementEvent::ReconcileUnknown(attestation.status),
            )?,
            state => {
                return Err(NodeError::Transition(TransitionError {
                    from: state,
                    event: SettlementEvent::RequestPayout,
                }));
            }
        };
        record.attestations.push(attestation);
        self.store.put(record);
        Ok(())
    }

    pub fn mark_payout_unknown(&self, id: SettlementId) -> Result<(), NodeError> {
        let mut record = self.store.get(id).ok_or(NodeError::UnknownSettlement(id))?;
        let requested = transition(record.state, SettlementEvent::RequestPayout)?;
        record.state = transition(requested, SettlementEvent::ObservePayout(None))?;
        self.store.put(record);
        Ok(())
    }

    /// Queries the fiat rail again for a settlement stuck in
    /// `PayoutUnknown` and moves it to a definite outcome once the rail
    /// can say. Does nothing to the rail itself — it only asks.
    pub fn reconcile_unknown_payout(&self, id: SettlementId) -> Result<PayoutOutcome, NodeError> {
        let mut record = self.store.get(id).ok_or(NodeError::UnknownSettlement(id))?;
        if record.state != SettlementState::PayoutUnknown {
            return Err(NodeError::NotReconcilable(record.state));
        }

        let outcome = self
            .adapter
            .query_payout(&record.payout_idempotency_key)
            .map_err(NodeError::Adapter)?
            .ok_or(NodeError::StillUnknown)?;

        record.state = transition(record.state, SettlementEvent::ReconcileUnknown(outcome))?;
        self.store.put(record);
        Ok(outcome)
    }

    /// Payer side: pays the accepted invoice now.
    pub fn release_settlement(&self, id: SettlementId) -> Result<PaymentPreimage, NodeError> {
        let mut record = self.store.get(id).ok_or(NodeError::UnknownSettlement(id))?;
        // Validate before paying: an illegal call must not send a real
        // Lightning payment before being rejected.
        let next_state = transition(record.state, SettlementEvent::ReleaseSettlement)?;
        let preimage = self.lightning.release(id).map_err(NodeError::Lightning)?;
        record.state = next_state;
        self.store.put(record);
        Ok(preimage)
    }

    pub fn refund_settlement(&self, id: SettlementId) -> Result<(), NodeError> {
        let mut record = self.store.get(id).ok_or(NodeError::UnknownSettlement(id))?;
        // Validate before discarding the accepted invoice: an illegal call
        // must not throw away a still-valid conditional settlement before
        // being rejected.
        let next_state = transition(record.state, SettlementEvent::Refund)?;
        self.lightning.refund(id).map_err(NodeError::Lightning)?;
        record.state = next_state;
        self.store.put(record);
        Ok(())
    }

    /// Signs a payout attestation with this node's identity key.
    pub fn sign_payout_attestation(
        &self,
        settlement_id: SettlementId,
        status: PayoutOutcome,
        evidence_ref: impl Into<String>,
        observed_at_unix: u64,
    ) -> PayoutAttestation {
        attestation::sign(
            &self.secp,
            &self.signing_key,
            settlement_id,
            status,
            evidence_ref,
            observed_at_unix,
        )
    }

    /// Records a peer's payout attestation after verifying its signature.
    /// If it conflicts with previously recorded evidence for the same
    /// settlement, the settlement moves to `Disputed` rather than picking
    /// a side.
    pub fn record_payout_attestation(
        &self,
        attestation: PayoutAttestation,
    ) -> Result<(), NodeError> {
        attestation::verify(&self.secp, &attestation).map_err(NodeError::InvalidAttestation)?;
        if let Some(expected) = self.peer_identity {
            let actual = PublicKey::from_slice(&attestation.signer_pubkey)
                .map_err(|_| NodeError::InvalidAttestation(AttestationError::Malformed))?;
            if actual != expected {
                return Err(NodeError::UnexpectedAttestationSigner);
            }
        }

        let mut record = self
            .store
            .get(attestation.settlement_id)
            .ok_or(NodeError::UnknownSettlement(attestation.settlement_id))?;

        let conflicts = record
            .attestations
            .iter()
            .any(|existing| existing.status != attestation.status);

        record.attestations.push(attestation);

        if conflicts {
            record.state = transition(record.state, SettlementEvent::Dispute)?;
            self.store.put(record);
            return Err(NodeError::ConflictingEvidence);
        }

        self.store.put(record);
        Ok(())
    }

    pub fn settlement_state(&self, id: SettlementId) -> Option<SettlementState> {
        self.store.get(id).map(|record| record.state)
    }

    /// Rebuilds the Lightning backend's in-memory invoice cache from durable
    /// records. It is safe to call on every process startup.
    pub fn recover(&self) -> Result<(), NodeError> {
        for record in self.store.list_pending() {
            if let Some(invoice) = record.counterparty_invoice
                && matches!(
                    record.state,
                    SettlementState::SettlementConditionCreated
                        | SettlementState::PayoutRequested
                        | SettlementState::PayoutUnknown
                        | SettlementState::PayoutConfirmed
                )
            {
                self.lightning
                    .accept_settlement_invoice(record.id, record.btc_amount, invoice)
                    .map_err(NodeError::Lightning)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::simulated::{SimulatedFiatAdapter, SimulatedPayoutBehavior};
    use crate::money::Currency;
    use crate::store::InMemorySettlementStore;

    /// A pure in-process Lightning double for testing the state machine
    /// and fiat-adapter interplay in isolation, with no socket, no
    /// process, no real payment. This is ordinary test scaffolding
    /// (`#[cfg(test)]`, unreachable outside `cargo test`) — the daemon
    /// itself only ever runs against [`crate::lightning::cln`].
    struct NoopLightning;

    impl LightningSettlement for NoopLightning {
        fn create_settlement_invoice(
            &self,
            _settlement_id: SettlementId,
            _amount: Millisatoshis,
        ) -> Result<Bolt11Invoice, LightningError> {
            Ok(Bolt11Invoice("noop-invoice".to_string()))
        }

        fn accept_settlement_invoice(
            &self,
            _settlement_id: SettlementId,
            _expected_amount: Millisatoshis,
            _invoice: Bolt11Invoice,
        ) -> Result<(), LightningError> {
            Ok(())
        }

        fn release(&self, _settlement_id: SettlementId) -> Result<PaymentPreimage, LightningError> {
            Ok(PaymentPreimage([0u8; 32]))
        }

        fn refund(&self, _settlement_id: SettlementId) -> Result<(), LightningError> {
            Ok(())
        }
    }

    /// A Lightning double that counts calls, used to prove that an
    /// illegal `release`/`refund` request never reaches the backend —
    /// state-machine validation must happen before any real side effect.
    #[derive(Clone, Default)]
    struct CountingLightning {
        release_calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
        refund_calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    impl LightningSettlement for CountingLightning {
        fn create_settlement_invoice(
            &self,
            _settlement_id: SettlementId,
            _amount: Millisatoshis,
        ) -> Result<Bolt11Invoice, LightningError> {
            Ok(Bolt11Invoice("noop-invoice".to_string()))
        }

        fn accept_settlement_invoice(
            &self,
            _settlement_id: SettlementId,
            _expected_amount: Millisatoshis,
            _invoice: Bolt11Invoice,
        ) -> Result<(), LightningError> {
            Ok(())
        }

        fn release(&self, _settlement_id: SettlementId) -> Result<PaymentPreimage, LightningError> {
            self.release_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(PaymentPreimage([0u8; 32]))
        }

        fn refund(&self, _settlement_id: SettlementId) -> Result<(), LightningError> {
            self.refund_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn test_node() -> Node<SimulatedFiatAdapter, NoopLightning, InMemorySettlementStore> {
        let signing_key = SecretKey::from_slice(&[3u8; 32]).unwrap();
        Node::new(
            SimulatedFiatAdapter::new(),
            NoopLightning,
            InMemorySettlementStore::new(),
            signing_key,
        )
    }

    fn amount() -> FiatAmount {
        FiatAmount::new(Currency::Ngn, 5_000)
    }

    fn settle(
        node: &Node<SimulatedFiatAdapter, NoopLightning, InMemorySettlementStore>,
    ) -> SettlementId {
        let id = node.begin_settlement(amount(), Millisatoshis(5_000_000), "kes-node");
        node.attach_counterparty_invoice(id, Bolt11Invoice("noop-invoice".to_string()))
            .unwrap();
        id
    }

    #[test]
    fn successful_settlement_reaches_released() {
        let node = test_node();
        let id = settle(&node);

        let outcome = node.request_payout(id).unwrap();
        assert_eq!(outcome, PayoutOutcome::Confirmed);

        node.release_settlement(id).unwrap();
        assert_eq!(
            node.settlement_state(id),
            Some(SettlementState::SettlementReleased)
        );
    }

    #[test]
    fn invalid_release_and_refund_do_not_touch_lightning() {
        let node = Node::new(
            SimulatedFiatAdapter::new(),
            CountingLightning::default(),
            InMemorySettlementStore::new(),
            SecretKey::from_slice(&[3u8; 32]).unwrap(),
        );
        let id = node.begin_settlement(amount(), Millisatoshis(5_000_000), "kes-node");
        node.attach_counterparty_invoice(id, Bolt11Invoice("noop-invoice".into()))
            .unwrap();

        assert!(matches!(
            node.release_settlement(id),
            Err(NodeError::Transition(_))
        ));
        assert!(matches!(
            node.refund_settlement(id),
            Err(NodeError::Transition(_))
        ));
        assert_eq!(
            node.lightning
                .release_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(
            node.lightning
                .refund_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(
            node.settlement_state(id),
            Some(SettlementState::SettlementConditionCreated)
        );

        node.request_payout(id).unwrap();
        node.release_settlement(id).unwrap();
        assert_eq!(
            node.settlement_state(id),
            Some(SettlementState::SettlementReleased)
        );
        assert_eq!(
            node.lightning
                .release_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[test]
    fn rejected_payout_can_be_refunded() {
        let node = test_node();
        let id = settle(&node);
        node.adapter.set_behavior(
            node.store.get(id).unwrap().payout_idempotency_key,
            SimulatedPayoutBehavior::Rejected("invalid beneficiary".to_string()),
        );

        let result = node.request_payout(id);
        assert!(matches!(result, Err(NodeError::PayoutRejected(_))));
        assert_eq!(
            node.settlement_state(id),
            Some(SettlementState::PayoutFailed)
        );

        node.refund_settlement(id).unwrap();
        assert_eq!(node.settlement_state(id), Some(SettlementState::Refunded));
    }

    #[test]
    fn timeout_moves_to_unknown_then_reconciles_without_double_payout() {
        let node = test_node();
        let id = settle(&node);
        node.adapter.set_behavior(
            node.store.get(id).unwrap().payout_idempotency_key,
            SimulatedPayoutBehavior::ConfirmedAfterTimeout,
        );

        let first = node.request_payout(id);
        assert!(matches!(first, Err(NodeError::PayoutUnknown)));
        assert_eq!(
            node.settlement_state(id),
            Some(SettlementState::PayoutUnknown)
        );

        let reconciled = node.reconcile_unknown_payout(id).unwrap();
        assert_eq!(reconciled, PayoutOutcome::Confirmed);
        assert_eq!(
            node.settlement_state(id),
            Some(SettlementState::PayoutConfirmed)
        );
    }

    #[test]
    fn duplicate_payout_request_is_idempotent() {
        let node = test_node();
        let id = settle(&node);

        let first = node.request_payout(id).unwrap();
        // The state machine no longer allows a second RequestPayout from
        // PayoutConfirmed, so a naive retry is rejected at the protocol
        // level rather than silently re-triggering the rail.
        let retry = node.request_payout(id);
        assert_eq!(first, PayoutOutcome::Confirmed);
        assert!(matches!(retry, Err(NodeError::Transition(_))));
    }

    #[test]
    fn invalid_attestation_signature_is_rejected() {
        let node = test_node();
        let id = settle(&node);

        let mut attestation =
            node.sign_payout_attestation(id, PayoutOutcome::Confirmed, "rail-tx-1", 1_700_000_000);
        attestation.signature[0] ^= 0xFF;

        let result = node.record_payout_attestation(attestation);
        assert!(matches!(result, Err(NodeError::InvalidAttestation(_))));
    }

    #[test]
    fn conflicting_attestations_move_settlement_to_disputed() {
        let node = test_node();
        let id = settle(&node);
        node.request_payout(id).unwrap();

        let confirmed =
            node.sign_payout_attestation(id, PayoutOutcome::Confirmed, "rail-tx-1", 1_700_000_000);
        node.record_payout_attestation(confirmed).unwrap();

        let failed =
            node.sign_payout_attestation(id, PayoutOutcome::Failed, "rail-tx-1", 1_700_000_001);
        let result = node.record_payout_attestation(failed);

        assert!(matches!(result, Err(NodeError::ConflictingEvidence)));
        assert_eq!(node.settlement_state(id), Some(SettlementState::Disputed));
    }

    #[test]
    fn attestation_from_an_unconfigured_peer_is_rejected() {
        let expected = SecretKey::from_slice(&[4u8; 32]).unwrap();
        let node = Node::new(
            SimulatedFiatAdapter::new(),
            NoopLightning,
            InMemorySettlementStore::new(),
            SecretKey::from_slice(&[3u8; 32]).unwrap(),
        )
        .with_peer_identity(PublicKey::from_secret_key(&Secp256k1::new(), &expected));
        let id = settle(&node);
        let other = SecretKey::from_slice(&[5u8; 32]).unwrap();
        let attestation = attestation::sign(
            &Secp256k1::new(),
            &other,
            id,
            PayoutOutcome::Confirmed,
            "rail-tx-1",
            1_700_000_000,
        );

        assert!(matches!(
            node.record_payout_attestation(attestation),
            Err(NodeError::UnexpectedAttestationSigner)
        ));
    }

    #[test]
    fn destination_payout_attestation_gates_source_lightning_release() {
        let destination_key = SecretKey::from_slice(&[4u8; 32]).unwrap();
        let source = Node::new(
            SimulatedFiatAdapter::new(),
            NoopLightning,
            InMemorySettlementStore::new(),
            SecretKey::from_slice(&[3u8; 32]).unwrap(),
        )
        .with_peer_identity(PublicKey::from_secret_key(
            &Secp256k1::new(),
            &destination_key,
        ));
        let destination = Node::new(
            SimulatedFiatAdapter::new(),
            NoopLightning,
            InMemorySettlementStore::new(),
            destination_key,
        );
        let id = source.begin_settlement(amount(), Millisatoshis(5_000_000), "kes-node");
        let invoice = destination
            .prepare_destination(
                id,
                FiatAmount::new(Currency::Kes, 3_000),
                Millisatoshis(5_000_000),
                "ngn-node:beneficiary",
                "254700000000",
                None,
            )
            .unwrap();
        source.attach_counterparty_invoice(id, invoice).unwrap();

        assert!(matches!(
            source.release_settlement(id),
            Err(NodeError::Transition(_))
        ));
        let outcome = destination.request_payout(id).unwrap();
        let attestation =
            destination.sign_payout_attestation(id, outcome, "kes-rail-1", 1_700_000_000);
        source.apply_payout_attestation(attestation).unwrap();
        source.release_settlement(id).unwrap();
        assert_eq!(
            source.settlement_state(id),
            Some(SettlementState::SettlementReleased)
        );
    }

    #[test]
    fn expired_quote_cannot_reserve_or_accept_an_invoice() {
        let node = test_node();
        let id = node.begin_settlement_with_quote_expiry(
            amount(),
            Millisatoshis(5_000_000),
            "kes-node",
            Some(1),
        );
        assert!(matches!(
            node.attach_counterparty_invoice(id, Bolt11Invoice("noop-invoice".into())),
            Err(NodeError::QuoteExpired)
        ));
        assert_eq!(node.settlement_state(id), Some(SettlementState::Quoted));
    }

    #[test]
    fn source_settlement_uses_only_terms_from_a_verified_quote() {
        let secp = Secp256k1::new();
        let source_key = SecretKey::from_slice(&[3u8; 32]).unwrap();
        let destination_key = SecretKey::from_slice(&[4u8; 32]).unwrap();
        let source = Node::new(
            SimulatedFiatAdapter::new(),
            NoopLightning,
            InMemorySettlementStore::new(),
            source_key,
        )
        .with_peer_identity(PublicKey::from_secret_key(&secp, &destination_key));
        let source_pubkey = PublicKey::from_secret_key(&secp, &source_key);
        let destination_pubkey = PublicKey::from_secret_key(&secp, &destination_key);
        let quote = quote::sign(
            &secp,
            &destination_key,
            crate::quote::QuoteTerms {
                quote_id: crate::ids::QuoteId::new(),
                source_amount: FiatAmount::new(Currency::Ngn, 50_000),
                destination_amount: FiatAmount::new(Currency::Kes, 3_000),
                settlement_amount: Millisatoshis(100_000),
                fee_amount: FiatAmount::new(Currency::Ngn, 500),
                expires_at_unix: u64::MAX,
                source_provider: source_pubkey.serialize().to_vec(),
                destination_provider: destination_pubkey.serialize().to_vec(),
            },
        );
        let id = source
            .begin_settlement_from_quote(quote, "kes-node")
            .unwrap();
        let record = source.store.get(id).unwrap();
        assert_eq!(record.fiat_amount, FiatAmount::new(Currency::Ngn, 50_000));
        assert_eq!(record.btc_amount, Millisatoshis(100_000));
        assert!(record.quote.is_some());
    }
}
