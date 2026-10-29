//! The HTTP surface between Safro nodes (peer-facing) and between a
//! provider and its own node (provider-facing). JSON over plain HTTP is
//! enough to prove the protocol logic for v0.1 — no protobuf/gRPC layer
//! yet.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::adapter::FiatAdapter;
use crate::attestation::PayoutAttestation;
use crate::ids::SettlementId;
use crate::lightning::{Bolt11Invoice, LightningSettlement};
use crate::money::{FiatAmount, Millisatoshis};
use crate::node::Node;
use crate::state::SettlementState;
use crate::store::SettlementStore;

pub struct AppState<A, L, S>
where
    A: FiatAdapter,
    L: LightningSettlement,
    S: SettlementStore,
{
    pub node: Arc<Node<A, L, S>>,
    /// Base URL of the counterparty node's API, e.g. `http://127.0.0.1:8082`.
    pub peer_addr: String,
    pub http: reqwest::Client,
}

impl<A, L, S> Clone for AppState<A, L, S>
where
    A: FiatAdapter,
    L: LightningSettlement,
    S: SettlementStore,
{
    fn clone(&self) -> Self {
        Self {
            node: self.node.clone(),
            peer_addr: self.peer_addr.clone(),
            http: self.http.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

type ApiError = (StatusCode, Json<ErrorBody>);

fn bad_request(message: impl Into<String>) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
}

fn not_found(message: impl Into<String>) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
}

fn parse_settlement_id(raw: &str) -> Result<SettlementId, ApiError> {
    SettlementId::from_hex(raw).ok_or_else(|| bad_request("settlement id is not valid hex"))
}

#[derive(Debug, Deserialize)]
struct CreateSettlementRequest {
    fiat_amount: FiatAmount,
    destination_amount: FiatAmount,
    btc_amount_msat: u64,
    counterparty: String,
    beneficiary: String,
    quote_expires_at_unix: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SettlementResponse {
    settlement_id: String,
    state: SettlementState,
}

/// Provider-facing: starts a settlement as the payer, fetching the
/// counterparty's invoice over the peer API before returning.
async fn create_settlement<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Json(req): Json<CreateSettlementRequest>,
) -> Result<Json<SettlementResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let btc_amount = Millisatoshis(req.btc_amount_msat);
    let quote_expires_at_unix = req.quote_expires_at_unix.or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|now| now.as_secs() + 300)
    });
    let id = state.node.begin_settlement_with_quote_expiry(
        req.fiat_amount,
        btc_amount,
        req.counterparty.clone(),
        quote_expires_at_unix,
    );

    let invoice_req = PrepareRequest {
        settlement_id: id.to_string(),
        destination_amount: req.destination_amount,
        btc_amount_msat: req.btc_amount_msat,
        counterparty: req.counterparty,
        beneficiary: req.beneficiary,
        quote_expires_at_unix,
    };
    let response = state
        .http
        .post(format!("{}/peer/prepare", state.peer_addr))
        .json(&invoice_req)
        .send()
        .await
        .map_err(|e| internal_error(format!("requesting invoice from peer: {e}")))?;

    if !response.status().is_success() {
        return Err(internal_error(format!(
            "peer rejected invoice request: {}",
            response.status()
        )));
    }

    let invoice_response: InvoiceResponse = response
        .json()
        .await
        .map_err(|e| internal_error(format!("parsing peer invoice response: {e}")))?;

    state
        .node
        .attach_counterparty_invoice(id, Bolt11Invoice(invoice_response.bolt11))
        .map_err(|e| internal_error(e.to_string()))?;

    let settlement_state = state
        .node
        .settlement_state(id)
        .expect("settlement was just created and updated");

    Ok(Json(SettlementResponse {
        settlement_id: id.to_string(),
        state: settlement_state,
    }))
}

async fn get_settlement<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<Json<SettlementResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    let settlement_state = state
        .node
        .settlement_state(settlement_id)
        .ok_or_else(|| not_found("unknown settlement"))?;
    Ok(Json(SettlementResponse {
        settlement_id: id,
        state: settlement_state,
    }))
}

async fn request_payout<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<Json<SettlementResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    state
        .node
        .request_payout(settlement_id)
        .map_err(|e| internal_error(e.to_string()))?;
    let settlement_state = state
        .node
        .settlement_state(settlement_id)
        .expect("settlement exists");
    Ok(Json(SettlementResponse {
        settlement_id: id,
        state: settlement_state,
    }))
}

async fn reconcile_payout<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<Json<SettlementResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    state
        .node
        .reconcile_unknown_payout(settlement_id)
        .map_err(|e| internal_error(e.to_string()))?;
    let settlement_state = state
        .node
        .settlement_state(settlement_id)
        .expect("settlement exists");
    Ok(Json(SettlementResponse {
        settlement_id: id,
        state: settlement_state,
    }))
}

async fn reconcile_peer_payout<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<Json<SettlementResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    let response = state
        .http
        .post(format!(
            "{}/peer/settlements/{settlement_id}/reconcile",
            state.peer_addr
        ))
        .send()
        .await
        .map_err(|e| internal_error(format!("reconciling peer payout: {e}")))?;
    if response.status() == StatusCode::ACCEPTED {
        return Ok(Json(SettlementResponse {
            settlement_id: id,
            state: SettlementState::PayoutUnknown,
        }));
    }
    if !response.status().is_success() {
        return Err(internal_error(format!(
            "peer reconciliation failed: {}",
            response.status()
        )));
    }
    let payout: PeerPayoutResponse = response
        .json()
        .await
        .map_err(|e| internal_error(format!("parsing peer attestation: {e}")))?;
    let attestation = payout
        .attestation
        .ok_or_else(|| internal_error("peer omitted payout attestation"))?;
    state
        .node
        .apply_payout_attestation(attestation)
        .map_err(|e| bad_request(e.to_string()))?;
    match state.node.settlement_state(settlement_id) {
        Some(SettlementState::PayoutConfirmed) => {
            state
                .node
                .release_settlement(settlement_id)
                .map_err(|e| internal_error(e.to_string()))?;
        }
        Some(SettlementState::PayoutFailed) => {
            state
                .node
                .refund_settlement(settlement_id)
                .map_err(|e| internal_error(e.to_string()))?;
        }
        _ => {
            return Err(internal_error(
                "unexpected settlement state after reconciliation",
            ));
        }
    }
    Ok(Json(SettlementResponse {
        settlement_id: id,
        state: state
            .node
            .settlement_state(settlement_id)
            .expect("settlement exists"),
    }))
}

async fn release_settlement<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<Json<SettlementResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    state
        .node
        .release_settlement(settlement_id)
        .map_err(|e| internal_error(e.to_string()))?;
    let settlement_state = state
        .node
        .settlement_state(settlement_id)
        .expect("settlement exists");
    Ok(Json(SettlementResponse {
        settlement_id: id,
        state: settlement_state,
    }))
}

/// Source-provider action: ask the destination provider to perform its fiat
/// payout, accept its signed outcome, then release the already-recorded
/// Lightning invoice only on confirmation.
async fn settle<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<Json<SettlementResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    let response = state
        .http
        .post(format!(
            "{}/peer/settlements/{settlement_id}/payout",
            state.peer_addr
        ))
        .send()
        .await
        .map_err(|e| internal_error(format!("requesting peer payout: {e}")))?;
    if !response.status().is_success() {
        return Err(internal_error(format!(
            "peer payout failed: {}",
            response.status()
        )));
    }
    if response.status() == StatusCode::ACCEPTED {
        state
            .node
            .mark_payout_unknown(settlement_id)
            .map_err(|e| internal_error(e.to_string()))?;
        let settlement_state = state
            .node
            .settlement_state(settlement_id)
            .expect("settlement exists");
        return Ok(Json(SettlementResponse {
            settlement_id: id,
            state: settlement_state,
        }));
    }
    let payout: PeerPayoutResponse = response
        .json()
        .await
        .map_err(|e| internal_error(format!("parsing peer attestation: {e}")))?;
    let attestation = payout
        .attestation
        .ok_or_else(|| internal_error("peer omitted payout attestation"))?;
    state
        .node
        .apply_payout_attestation(attestation)
        .map_err(|e| bad_request(e.to_string()))?;
    match state.node.settlement_state(settlement_id) {
        Some(SettlementState::PayoutConfirmed) => {
            state
                .node
                .release_settlement(settlement_id)
                .map_err(|e| internal_error(e.to_string()))?;
        }
        Some(SettlementState::PayoutFailed) => {
            state
                .node
                .refund_settlement(settlement_id)
                .map_err(|e| internal_error(e.to_string()))?;
        }
        Some(current) => {
            return Err(internal_error(format!(
                "unexpected payout state: {current:?}"
            )));
        }
        None => return Err(not_found("unknown settlement")),
    }
    let settlement_state = state
        .node
        .settlement_state(settlement_id)
        .expect("settlement exists");
    Ok(Json(SettlementResponse {
        settlement_id: id,
        state: settlement_state,
    }))
}

async fn refund_settlement<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<Json<SettlementResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    state
        .node
        .refund_settlement(settlement_id)
        .map_err(|e| internal_error(e.to_string()))?;
    let settlement_state = state
        .node
        .settlement_state(settlement_id)
        .expect("settlement exists");
    Ok(Json(SettlementResponse {
        settlement_id: id,
        state: settlement_state,
    }))
}

#[derive(Debug, Serialize, Deserialize)]
struct InvoiceRequest {
    settlement_id: String,
    btc_amount_msat: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct InvoiceResponse {
    bolt11: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PrepareRequest {
    settlement_id: String,
    destination_amount: FiatAmount,
    btc_amount_msat: u64,
    counterparty: String,
    beneficiary: String,
    quote_expires_at_unix: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PeerPayoutResponse {
    attestation: Option<PayoutAttestation>,
}

/// Peer-facing: a counterparty is asking us (the payee) for an invoice.
async fn create_invoice<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Json(req): Json<InvoiceRequest>,
) -> Result<Json<InvoiceResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&req.settlement_id)?;
    let invoice = state
        .node
        .create_settlement_invoice(settlement_id, Millisatoshis(req.btc_amount_msat))
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(InvoiceResponse { bolt11: invoice.0 }))
}

async fn prepare_destination<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Json(req): Json<PrepareRequest>,
) -> Result<Json<InvoiceResponse>, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let id = parse_settlement_id(&req.settlement_id)?;
    let counterparty = format!("{}:{}", req.counterparty, req.beneficiary);
    let invoice = state
        .node
        .prepare_destination(
            id,
            req.destination_amount,
            Millisatoshis(req.btc_amount_msat),
            counterparty,
            req.beneficiary,
            req.quote_expires_at_unix,
        )
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(InvoiceResponse { bolt11: invoice.0 }))
}

async fn peer_payout<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<PeerPayoutResponse>), ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    let outcome = match state.node.request_payout(settlement_id) {
        Ok(outcome) => outcome,
        Err(crate::node::NodeError::PayoutUnknown) => {
            return Ok((
                StatusCode::ACCEPTED,
                Json(PeerPayoutResponse { attestation: None }),
            ));
        }
        Err(error) => return Err(bad_request(error.to_string())),
    };
    let attestation = state.node.sign_payout_attestation(
        settlement_id,
        outcome,
        format!("simulated-rail-{settlement_id}"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| internal_error(e.to_string()))?
            .as_secs(),
    );
    Ok((
        StatusCode::OK,
        Json(PeerPayoutResponse {
            attestation: Some(attestation),
        }),
    ))
}

async fn peer_reconcile_payout<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<PeerPayoutResponse>), ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    let settlement_id = parse_settlement_id(&id)?;
    let outcome = match state.node.reconcile_unknown_payout(settlement_id) {
        Ok(outcome) => outcome,
        Err(crate::node::NodeError::StillUnknown) => {
            return Ok((
                StatusCode::ACCEPTED,
                Json(PeerPayoutResponse { attestation: None }),
            ));
        }
        Err(error) => return Err(bad_request(error.to_string())),
    };
    let attestation = state.node.sign_payout_attestation(
        settlement_id,
        outcome,
        format!("simulated-rail-{settlement_id}"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| internal_error(e.to_string()))?
            .as_secs(),
    );
    Ok((
        StatusCode::OK,
        Json(PeerPayoutResponse {
            attestation: Some(attestation),
        }),
    ))
}

/// Peer-facing: a counterparty is sending us a signed payout attestation.
async fn receive_attestation<A, L, S>(
    State(state): State<AppState<A, L, S>>,
    Json(attestation): Json<PayoutAttestation>,
) -> Result<StatusCode, ApiError>
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    state
        .node
        .record_payout_attestation(attestation)
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(StatusCode::OK)
}

pub fn router<A, L, S>(state: AppState<A, L, S>) -> Router
where
    A: FiatAdapter + 'static,
    L: LightningSettlement + 'static,
    S: SettlementStore + 'static,
{
    Router::new()
        .route("/settlements", post(create_settlement::<A, L, S>))
        .route("/settlements/:id", get(get_settlement::<A, L, S>))
        .route("/settlements/:id/payout", post(request_payout::<A, L, S>))
        .route(
            "/settlements/:id/reconcile",
            post(reconcile_payout::<A, L, S>),
        )
        .route(
            "/settlements/:id/reconcile-peer",
            post(reconcile_peer_payout::<A, L, S>),
        )
        .route(
            "/settlements/:id/release",
            post(release_settlement::<A, L, S>),
        )
        .route("/settlements/:id/settle", post(settle::<A, L, S>))
        .route(
            "/settlements/:id/refund",
            post(refund_settlement::<A, L, S>),
        )
        .route("/peer/invoices", post(create_invoice::<A, L, S>))
        .route("/peer/prepare", post(prepare_destination::<A, L, S>))
        .route("/peer/settlements/:id/payout", post(peer_payout::<A, L, S>))
        .route(
            "/peer/settlements/:id/reconcile",
            post(peer_reconcile_payout::<A, L, S>),
        )
        .route("/peer/attestations", post(receive_attestation::<A, L, S>))
        .with_state(state)
}
