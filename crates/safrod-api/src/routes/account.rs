use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use safrod_core::apikey::Scope;
use safrod_storage::StorageError;
use serde::Deserialize;
use serde_json::json;

use super::query;
use crate::AppState;
use crate::auth::{authenticate, authorize};
use crate::engine::blocking;
use crate::error::ApiError;
use crate::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/project", get(project))
        .route("/v1/balances", get(balances))
        .route("/v1/node", get(node))
        .route("/v1/events", get(list_events))
        .route("/v1/events/:id", get(get_event))
}

async fn project(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let auth = authenticate(&state, &headers).await?;
    let project = state.db.get_project(&auth.project_id).await?;
    Ok(json::ok(json!({
        "object": "project", "id": project.id, "name": project.name,
        "network": project.network, "created_at": project.created_at,
        "scopes": auth.scopes,
    })))
}

/// Three separate sections so simulated money is never summed with real
/// Bitcoin-backed amounts.
async fn balances(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::PaymentsRead).await?;
    let totals = state.db.project_totals(&auth.project_id).await?;
    let sandbox = state.db.sandbox_balances(&auth.project_id).await?;
    Ok(json::ok(json!({
        "object": "balances",
        "network": state.config.network.as_str(),
        "ledger": {
            "description": "Totals from settled records in this project's ledger, not node balances.",
            "lightning_received_msat": totals.lightning_received_msat,
            "lightning_sent_msat": totals.lightning_sent_msat,
            "lightning_fees_msat": totals.lightning_fees_msat,
            "lightning_in_flight_msat": totals.lightning_in_flight_msat,
            "onchain_confirmed_sat": totals.onchain_confirmed_sat,
            "onchain_unconfirmed_sat": totals.onchain_unconfirmed_sat,
            "simulated": false,
        },
        "simulated_fiat": {
            "description": "Synthetic sandbox balances. Not real money.",
            "balances": sandbox.iter().map(|(c, m)| json!({ "currency": c, "minor_units": m })).collect::<Vec<_>>(),
            "simulated": true,
        },
    })))
}

/// Node-wide, not per project: every project shares one Lightning node.
async fn node(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    authorize(&state, &headers, Scope::PaymentsRead).await?;
    let (info, channels, balances) =
        blocking(&state.ln, |ln| (ln.info(), ln.channels(), ln.balances())).await?;
    let height = match &state.rpc {
        Some(rpc) => rpc.block_count().await.ok(),
        None => None,
    };
    Ok(json::ok(json!({
        "object": "node",
        "info": info,
        "channels": channels,
        "balances": balances,
        "bitcoin_core_height": height,
    })))
}

#[derive(Deserialize, Default)]
struct EventQuery {
    after: Option<i64>,
    #[serde(rename = "type")]
    kind: Option<String>,
    limit: Option<i64>,
}

async fn list_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    q: Result<Query<EventQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::EventsRead).await?;
    let q = query(q)?;
    let page = json::Page::from_query(q.limit)?;
    let rows = state
        .db
        .list_events(&auth.project_id, q.after, q.kind.as_deref(), page.fetch())
        .await?;
    let has_more = rows.len() as i64 > page.limit;
    let rows: Vec<_> = rows.into_iter().take(page.limit as usize).collect();
    let next_after = if has_more {
        rows.last().map(|r| r.seq)
    } else {
        None
    };
    Ok(json::ok(json!({
        "object": "list",
        "data": rows.iter().map(json::event).collect::<Vec<_>>(),
        "has_more": has_more,
        "next_after": next_after,
    })))
}

async fn get_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::EventsRead).await?;
    let event = state
        .db
        .get_event(&auth.project_id, &id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("event"),
            other => other.into(),
        })?;
    Ok(json::ok(json::event(&event)))
}
