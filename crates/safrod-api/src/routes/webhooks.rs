use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use rand::RngCore;
use safrod_core::apikey::Scope;
use safrod_storage::StorageError;
use safrod_webhooks::validate_url;
use serde::Deserialize;
use serde_json::json;

use super::{ListQuery, query};
use crate::AppState;
use crate::auth::authorize;
use crate::error::ApiError;
use crate::json::{self, Page};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/webhook-endpoints",
            post(create_endpoint).get(list_endpoints),
        )
        .route("/v1/webhook-endpoints/:id", delete(disable_endpoint))
        .route("/v1/webhook-endpoints/:id/test", post(test_endpoint))
        .route("/v1/webhook-deliveries", get(list_deliveries))
        .route("/v1/webhook-deliveries/:id/replay", post(replay_delivery))
}

#[derive(Deserialize)]
struct EndpointRequest {
    url: String,
}

async fn create_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::WebhooksWrite).await?;
    let req: EndpointRequest = json::parse(&body)?;
    if req.url.len() > 2048 {
        return Err(ApiError::validation("url is too long"));
    }
    validate_url(&req.url, state.config.webhook_allow_private)
        .map_err(|e| ApiError::validation(format!("webhook url rejected: {e}")))?;
    let mut raw = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut raw);
    let secret = format!("whsec_{}", hex::encode(raw));
    let endpoint = state
        .db
        .create_webhook_endpoint(&auth.project_id, &req.url, &secret)
        .await?;
    let mut body = json::endpoint(&endpoint);
    body["secret"] = json!(secret);
    body["note"] = json!("The signing secret is shown once.");
    Ok(json::created(true, body))
}

async fn list_endpoints(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::WebhooksWrite).await?;
    let rows = state.db.list_webhook_endpoints(&auth.project_id).await?;
    Ok(json::ok(json!({
        "object": "list",
        "data": rows.iter().map(json::endpoint).collect::<Vec<_>>(),
        "has_more": false,
        "next_cursor": null,
    })))
}

async fn disable_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::WebhooksWrite).await?;
    state
        .db
        .disable_webhook_endpoint(&auth.project_id, &id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("webhook endpoint"),
            other => other.into(),
        })?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Emits a `webhook.test` event to every enabled endpoint of the project.
async fn test_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::WebhooksWrite).await?;
    let endpoints = state.db.list_webhook_endpoints(&auth.project_id).await?;
    if !endpoints.iter().any(|e| e.id == id && e.enabled) {
        return Err(ApiError::not_found("webhook endpoint"));
    }
    let event = state
        .db
        .emit_standalone(
            &auth.project_id,
            "webhook.test",
            &id,
            json!({ "endpoint_id": id }),
            false,
        )
        .await?;
    Ok(json::created(true, json::event(&event)))
}

async fn list_deliveries(
    State(state): State<AppState>,
    headers: HeaderMap,
    q: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::WebhooksWrite).await?;
    let page = Page::from_query(query(q)?.limit)?;
    let rows = state
        .db
        .list_deliveries(&auth.project_id, page.fetch())
        .await?;
    Ok(json::ok(page.wrap(rows, json::delivery, |r| r.id.clone())))
}

async fn replay_delivery(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let auth = authorize(&state, &headers, Scope::WebhooksWrite).await?;
    state
        .db
        .replay_delivery(&auth.project_id, &id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => ApiError::not_found("delivery"),
            other => other.into(),
        })?;
    Ok(json::ok(
        json!({ "id": id, "status": "pending", "note": "Receivers must deduplicate on the event id." }),
    ))
}
