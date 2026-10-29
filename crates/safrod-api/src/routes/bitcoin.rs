use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use safrod_core::apikey::Scope;
use safrod_core::ids::TransactionId;
use safrod_storage::StorageError;
use serde::Deserialize;

use super::{ListQuery, id, query};
use crate::AppState;
use crate::auth::authorize;
use crate::engine::blocking;
use crate::error::ApiError;
use crate::json::{self, Page};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/bitcoin/addresses", post(create_address))
        .route("/v1/bitcoin/transactions", get(list_transactions))
        .route("/v1/bitcoin/transactions/:id", get(get_transaction))
}

#[derive(Deserialize, Default)]
struct AddressRequest {
    #[serde(default)]
    label: String,
    required_confirmations: Option<i32>,
}

/// Addresses come from the node's own wallet. The deposit watcher then tracks
/// payments to them via Bitcoin Core.
async fn create_address(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::BitcoinWrite).await?;
    let req: AddressRequest = json::parse_or_default(&body)?;
    if req.label.len() > 100 {
        return Err(ApiError::validation("label is too long"));
    }
    let confirmations = req
        .required_confirmations
        .unwrap_or(state.config.default_confirmations);
    if !(1..=100).contains(&confirmations) {
        return Err(ApiError::validation(
            "required_confirmations must be between 1 and 100",
        ));
    }
    let address = blocking(&state.ln, |ln| ln.new_onchain_address()).await??;
    let record = state
        .db
        .create_address(&auth.project_id, &address, &req.label, confirmations)
        .await?;
    Ok(json::created(true, json::address(&record)))
}

async fn get_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(raw): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::BitcoinRead).await?;
    let id: TransactionId = id(&raw, "transaction")?;
    let tx = state
        .db
        .get_onchain_transaction(&auth.project_id, id.as_str())
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("transaction"),
            other => other.into(),
        })?;
    let address = state.db.address_for_transaction(&tx.address_id).await?;
    Ok(json::ok(json::onchain_tx(&tx, &address)))
}

async fn list_transactions(
    State(state): State<AppState>,
    headers: HeaderMap,
    q: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::BitcoinRead).await?;
    let page = Page::from_query(query(q)?.limit)?;
    let rows = state
        .db
        .list_onchain_transactions(&auth.project_id, page.fetch())
        .await?;
    let mut data = Vec::new();
    for tx in &rows {
        data.push((
            tx.clone(),
            state.db.address_for_transaction(&tx.address_id).await?,
        ));
    }
    Ok(json::ok(page.wrap(
        data,
        |(tx, address)| json::onchain_tx(tx, address),
        |(tx, _)| tx.id.clone(),
    )))
}
