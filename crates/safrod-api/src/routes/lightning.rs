use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use chrono::{TimeZone, Utc};
use safrod_core::apikey::Scope;
use safrod_core::ids::{InvoiceId, PaymentId};
use safrod_core::money::{MAX_SATS, Millisatoshis, Satoshis};
use safrod_lightning::NewInvoice as NodeInvoice;
use safrod_storage::StorageError;
use safrod_storage::invoices::NewInvoice;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ListQuery, id, query};
use crate::AppState;
use crate::auth::authorize;
use crate::engine::{self, SendRequest, blocking};
use crate::error::ApiError;
use crate::json::{self, Page};

const MAX_INVOICE_EXPIRY_SECS: u32 = 7 * 24 * 3600;
const MAX_DESCRIPTION_BYTES: usize = 639;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/lightning/invoices",
            post(create_invoice).get(list_invoices),
        )
        .route("/v1/lightning/invoices/:id", get(get_invoice))
        .route(
            "/v1/lightning/payments",
            post(create_payment).get(list_payments),
        )
        .route("/v1/lightning/payments/:id", get(get_payment))
        .route(
            "/v1/lightning/payments/:id/reconcile",
            post(reconcile_payment),
        )
}

#[derive(Deserialize)]
struct InvoiceRequest {
    amount_msat: Option<u64>,
    amount_sat: Option<u64>,
    #[serde(default)]
    description: String,
    expiry_secs: Option<u32>,
    #[serde(default)]
    metadata: Option<Value>,
}

fn invoice_amount(req: &InvoiceRequest) -> Result<u64, ApiError> {
    match (req.amount_msat, req.amount_sat) {
        (Some(_), Some(_)) => Err(ApiError::validation(
            "provide either amount_msat or amount_sat, not both",
        )),
        (None, None) => Err(ApiError::validation(
            "amount_msat or amount_sat is required",
        )),
        (None, Some(sat)) => {
            let sats =
                Satoshis::from_request(sat).map_err(|e| ApiError::validation(e.to_string()))?;
            sats.to_msat()
                .map(|m| m.0)
                .ok_or_else(|| ApiError::validation("amount is out of range"))
        }
        (Some(msat), None) => {
            if msat == 0 || Millisatoshis(msat).to_sats_floor().0 > MAX_SATS {
                return Err(ApiError::validation("amount_msat is out of range"));
            }
            Ok(msat)
        }
    }
}

fn metadata(raw: Option<Value>) -> Result<Value, ApiError> {
    match raw {
        None => Ok(json!({})),
        Some(v @ Value::Object(_)) if v.to_string().len() <= 4096 => Ok(v),
        Some(_) => Err(ApiError::validation(
            "metadata must be an object under 4 KiB",
        )),
    }
}

async fn create_invoice(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::InvoicesWrite).await?;
    let req: InvoiceRequest = json::parse(&body)?;
    let amount_msat = invoice_amount(&req)?;
    if req.description.len() > MAX_DESCRIPTION_BYTES {
        return Err(ApiError::validation("description is too long"));
    }
    let expiry_secs = req.expiry_secs.unwrap_or(3600);
    if expiry_secs == 0 || expiry_secs > MAX_INVOICE_EXPIRY_SECS {
        return Err(ApiError::validation(
            "expiry_secs must be between 1 and 604800",
        ));
    }
    let metadata = metadata(req.metadata)?;
    let idem = json::idempotency_key(&headers, false)?;

    if let Some(key) = &idem {
        if let Some(existing) = state
            .db
            .find_invoice_by_idempotency_key(&auth.project_id, key)
            .await?
        {
            if existing.amount_msat as u64 != amount_msat || existing.description != req.description
            {
                return Err(StorageError::IdempotencyMismatch.into());
            }
            return Ok(json::created(false, json::invoice(&existing)));
        }
    }

    let node_request = NodeInvoice {
        amount_msat,
        description: req.description.clone(),
        expiry_secs,
    };
    let created = blocking(&state.ln, move |ln| ln.create_invoice(&node_request)).await??;
    let expires_at = Utc
        .timestamp_opt(created.expires_at_unix as i64, 0)
        .single()
        .ok_or_else(ApiError::internal)?;
    let inserted = state
        .db
        .insert_invoice(NewInvoice {
            id: InvoiceId::new(),
            project_id: auth.project_id.clone(),
            idempotency_key: idem.clone(),
            amount_msat,
            description: req.description,
            bolt11: created.bolt11,
            payment_hash: created.payment_hash,
            expires_at,
            metadata,
        })
        .await;
    match inserted {
        Ok(invoice) => Ok(json::created(true, json::invoice(&invoice))),
        // Two concurrent requests with one key: the loser returns the winner's invoice.
        Err(StorageError::Conflict) if idem.is_some() => {
            let existing = state
                .db
                .find_invoice_by_idempotency_key(
                    &auth.project_id,
                    idem.as_deref().unwrap_or_default(),
                )
                .await?
                .ok_or_else(ApiError::internal)?;
            Ok(json::created(false, json::invoice(&existing)))
        }
        Err(e) => Err(e.into()),
    }
}

async fn get_invoice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(raw): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::InvoicesRead).await?;
    let id: InvoiceId = id(&raw, "invoice")?;
    let invoice = state
        .db
        .get_invoice(&auth.project_id, id.as_str())
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("invoice"),
            other => other.into(),
        })?;
    Ok(json::ok(json::invoice(&invoice)))
}

async fn list_invoices(
    State(state): State<AppState>,
    headers: HeaderMap,
    q: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::InvoicesRead).await?;
    let q = query(q)?;
    let page = Page::from_query(q.limit)?;
    let rows = state
        .db
        .list_invoices(&auth.project_id, q.starting_after.as_deref(), page.fetch())
        .await?;
    Ok(json::ok(page.wrap(rows, json::invoice, |r| r.id.clone())))
}

#[derive(Deserialize)]
struct PaymentRequest {
    bolt11: String,
    amount_msat: Option<u64>,
    #[serde(default)]
    metadata: Option<Value>,
}

fn reveal_preimage(state: &AppState) -> bool {
    state.config.network == safrod_core::network::Network::Regtest
}

async fn create_payment(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::PaymentsWrite).await?;
    let req: PaymentRequest = json::parse(&body)?;
    let idem = json::idempotency_key(&headers, true)?.unwrap_or_default();
    if req.bolt11.len() > 2048 {
        return Err(ApiError::validation("bolt11 is too long"));
    }
    let result = engine::send_payment(
        &state,
        &auth.project_id,
        &idem,
        SendRequest {
            bolt11: req.bolt11,
            amount_msat: req.amount_msat,
            metadata: metadata(req.metadata)?,
        },
    )
    .await?;
    Ok(json::created(
        result.created,
        json::payment(&result.payment, reveal_preimage(&state)),
    ))
}

async fn get_payment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(raw): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::PaymentsRead).await?;
    let id: PaymentId = id(&raw, "payment")?;
    let payment = state
        .db
        .get_payment(&auth.project_id, id.as_str())
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("payment"),
            other => other.into(),
        })?;
    Ok(json::ok(json::payment(&payment, reveal_preimage(&state))))
}

async fn list_payments(
    State(state): State<AppState>,
    headers: HeaderMap,
    q: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::PaymentsRead).await?;
    let q = query(q)?;
    let page = Page::from_query(q.limit)?;
    let rows = state
        .db
        .list_payments(&auth.project_id, q.starting_after.as_deref(), page.fetch())
        .await?;
    let reveal = reveal_preimage(&state);
    Ok(json::ok(page.wrap(
        rows,
        |r| json::payment(r, reveal),
        |r| r.id.clone(),
    )))
}

/// Asks the node what happened to a payment that has not reached a final
/// state. Safe to call repeatedly.
async fn reconcile_payment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(raw): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::PaymentsWrite).await?;
    let id: PaymentId = id(&raw, "payment")?;
    let payment = state
        .db
        .get_payment(&auth.project_id, id.as_str())
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("payment"),
            other => other.into(),
        })?;
    let after = engine::reconcile_payment(&state, &payment).await?;
    Ok(json::ok(json::payment(&after, reveal_preimage(&state))))
}
