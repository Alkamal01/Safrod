//! Simulated fiat and FX. Every response here is labelled `simulated_fiat`.
//! The only real part is the optional Bitcoin leg: when a settlement invoice
//! is supplied it is paid over regtest Lightning, but only after the simulated
//! destination payout is confirmed.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use chrono::{Duration, Utc};
use safrod_core::apikey::Scope;
use safrod_core::state::PayoutStatus;
use safrod_sandbox::{Currency, Scenario, quote};
use safrod_storage::StorageError;
use safrod_storage::models::PayoutRecord;
use safrod_storage::sandbox::NewQuote;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::authorize;
use crate::engine::{self, SendRequest, blocking};
use crate::error::ApiError;
use crate::json;

const MAX_TOPUP_MINOR: u64 = 1_000_000_000_000;
const QUOTE_TTL_SECS: i64 = 300;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/sandbox/balances", get(balances).post(topup))
        .route("/v1/sandbox/quotes", post(create_quote))
        .route("/v1/sandbox/quotes/:id", get(get_quote))
        .route("/v1/sandbox/payouts", post(create_payout))
        .route("/v1/sandbox/payouts/:id", get(get_payout))
        .route("/v1/sandbox/payouts/:id/reconcile", post(reconcile_payout))
}

fn currency(raw: &str) -> Result<Currency, ApiError> {
    raw.parse()
        .map_err(|e: safrod_sandbox::SandboxError| ApiError::validation(e.to_string()))
}

#[derive(Deserialize)]
struct TopUp {
    currency: String,
    minor_units: u64,
}

async fn topup(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::SandboxWrite).await?;
    let req: TopUp = json::parse(&body)?;
    let cur = currency(&req.currency)?;
    if req.minor_units == 0 || req.minor_units > MAX_TOPUP_MINOR {
        return Err(ApiError::validation("minor_units is out of range"));
    }
    state
        .db
        .credit_sandbox_balance(&auth.project_id, cur.code(), req.minor_units)
        .await?;
    balances_response(&state, &auth.project_id).await
}

async fn balances(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::SandboxWrite).await?;
    balances_response(&state, &auth.project_id).await
}

async fn balances_response(state: &AppState, project_id: &str) -> Result<Response, ApiError> {
    let rows = state.db.sandbox_balances(project_id).await?;
    Ok(json::ok(json!({
        "object": "sandbox_balances",
        "balances": rows.iter().map(|(c, m)| json!({ "currency": c, "minor_units": m })).collect::<Vec<_>>(),
        "simulated_fiat": true,
    })))
}

#[derive(Deserialize)]
struct QuoteAmount {
    currency: String,
    minor_units: u64,
}

#[derive(Deserialize)]
struct QuoteRequest {
    source: QuoteAmount,
    target_currency: String,
}

async fn create_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::SandboxWrite).await?;
    let req: QuoteRequest = json::parse(&body)?;
    let source = currency(&req.source.currency)?;
    let target = currency(&req.target_currency)?;
    let calc = quote(source, target, req.source.minor_units)
        .map_err(|e| ApiError::validation(e.to_string()))?;
    let record = state
        .db
        .insert_quote(NewQuote {
            project_id: auth.project_id,
            source_currency: source.code().into(),
            target_currency: target.code().into(),
            source_minor: req.source.minor_units,
            target_minor: calc.target_minor,
            rate_ppm: calc.rate_ppm,
            btc_amount_msat: calc.btc_amount_msat,
            expires_at: Utc::now() + Duration::seconds(QUOTE_TTL_SECS),
        })
        .await?;
    Ok(json::created(true, json::quote(&record)))
}

async fn get_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::SandboxWrite).await?;
    let record = state
        .db
        .get_quote(&auth.project_id, &id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("quote"),
            other => other.into(),
        })?;
    Ok(json::ok(json::quote(&record)))
}

#[derive(Deserialize)]
struct PayoutRequest {
    quote_id: String,
    scenario: String,
    settlement_invoice: Option<String>,
}

async fn create_payout(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::SandboxWrite).await?;
    let req: PayoutRequest = json::parse(&body)?;
    let idem = json::idempotency_key(&headers, true)?.unwrap_or_default();
    let scenario: Scenario = req
        .scenario
        .parse()
        .map_err(|e: safrod_sandbox::SandboxError| ApiError::validation(e.to_string()))?;
    let quote_record = state
        .db
        .get_quote(&auth.project_id, &req.quote_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("quote"),
            other => other.into(),
        })?;

    if let Some(invoice) = &req.settlement_invoice {
        let target = invoice.clone();
        let parsed = blocking(&state.ln, move |ln| ln.parse_invoice(&target)).await??;
        if let Some(amount) = parsed.amount_msat {
            if amount as i64 != quote_record.btc_amount_msat {
                return Err(ApiError::validation(
                    "settlement invoice amount does not match the quote's BTC amount",
                ));
            }
        }
    }

    let (payout, created) = state
        .db
        .reserve_payout(
            &auth.project_id,
            &req.quote_id,
            &idem,
            scenario.as_str(),
            req.settlement_invoice.as_deref(),
        )
        .await?;
    if !created {
        return Ok(json::created(false, json::payout(&payout)));
    }
    let finished = run_payout(&state, &auth.project_id, payout, scenario).await?;
    Ok(json::created(true, json::payout(&finished)))
}

async fn run_payout(
    state: &AppState,
    project_id: &str,
    payout: PayoutRecord,
    scenario: Scenario,
) -> Result<PayoutRecord, ApiError> {
    let (pending, _) = state
        .db
        .advance_payout(&payout.id, PayoutStatus::Pending)
        .await?;
    match scenario {
        Scenario::Failure => Ok(state
            .db
            .advance_payout(&pending.id, PayoutStatus::Failed)
            .await?
            .0),
        // A timeout does not prove the destination failed to pay out, so the
        // payout stays `unknown` and no Bitcoin is released until reconciled.
        Scenario::Timeout => Ok(state
            .db
            .advance_payout(&pending.id, PayoutStatus::Unknown)
            .await?
            .0),
        Scenario::Success => complete_success(state, project_id, pending).await,
    }
}

async fn complete_success(
    state: &AppState,
    project_id: &str,
    payout: PayoutRecord,
) -> Result<PayoutRecord, ApiError> {
    let (done, _) = state
        .db
        .advance_payout(&payout.id, PayoutStatus::Succeeded)
        .await?;
    let Some(invoice) = payout.settlement_bolt11.clone() else {
        return Ok(done);
    };
    let quote_record = state.db.get_quote(project_id, &done.quote_id).await?;
    let sent = engine::send_payment(
        state,
        project_id,
        &format!("sandbox-payout:{}", done.id),
        SendRequest {
            bolt11: invoice,
            amount_msat: Some(quote_record.btc_amount_msat as u64),
            metadata: json!({ "sandbox_payout_id": done.id }),
        },
    )
    .await?;
    state
        .db
        .attach_payment_to_payout(&done.id, &sent.payment.id)
        .await?;
    Ok(state.db.get_payout(project_id, &done.id).await?)
}

async fn get_payout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::SandboxWrite).await?;
    let payout = fetch_payout(&state, &auth.project_id, &id).await?;
    Ok(json::ok(
        payout_with_payment(&state, &auth.project_id, &payout).await?,
    ))
}

async fn fetch_payout(
    state: &AppState,
    project_id: &str,
    id: &str,
) -> Result<PayoutRecord, ApiError> {
    state
        .db
        .get_payout(project_id, id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("payout"),
            other => other.into(),
        })
}

async fn payout_with_payment(
    state: &AppState,
    project_id: &str,
    payout: &PayoutRecord,
) -> Result<Value, ApiError> {
    let mut body = json::payout(payout);
    if let Some(payment_id) = &payout.payment_id {
        let payment = state.db.get_payment(project_id, payment_id).await?;
        body["payment"] = json::payment(
            &payment,
            state.config.network == safrod_core::network::Network::Regtest,
        );
    }
    Ok(body)
}

#[derive(Deserialize)]
struct ReconcileRequest {
    outcome: String,
}

/// Resolves a payout left `unknown` by a simulated timeout. Stands in for the
/// destination provider later confirming or denying that it paid out.
async fn reconcile_payout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::SandboxWrite).await?;
    let req: ReconcileRequest = json::parse(&body)?;
    let payout = fetch_payout(&state, &auth.project_id, &id).await?;
    if payout.status != PayoutStatus::Unknown.as_str() {
        return Err(ApiError::conflict(
            "invalid_state",
            "only an unknown payout can be reconciled",
        ));
    }
    let resolved = match req.outcome.as_str() {
        "success" => complete_success(&state, &auth.project_id, payout).await?,
        "failure" => {
            state
                .db
                .advance_payout(&payout.id, PayoutStatus::Failed)
                .await?
                .0
        }
        _ => return Err(ApiError::validation("outcome must be success or failure")),
    };
    Ok(json::ok(
        payout_with_payment(&state, &auth.project_id, &resolved).await?,
    ))
}
