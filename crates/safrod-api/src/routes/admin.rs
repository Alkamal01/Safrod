use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use safrod_core::apikey::{self, Scope};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::auth::require_admin;
use crate::engine::blocking;
use crate::error::ApiError;
use crate::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/projects", post(create_project))
        .route(
            "/v1/projects/:project_id/api-keys",
            post(create_key).get(list_keys),
        )
        .route(
            "/v1/projects/:project_id/api-keys/:key_id",
            delete(revoke_key),
        )
        .route("/v1/dev/mine", post(mine))
        .route("/v1/dev/status", get(dev_status))
}

#[derive(Deserialize)]
struct NewProject {
    name: String,
}

#[derive(Deserialize, Default)]
struct NewKey {
    label: Option<String>,
    scopes: Option<Vec<String>>,
}

fn key_label_for(network: &safrod_core::network::Network) -> &'static str {
    match network {
        safrod_core::network::Network::Bitcoin => "live",
        _ => "test",
    }
}

fn parse_scopes(requested: Option<Vec<String>>) -> Result<Vec<String>, ApiError> {
    match requested {
        None => Ok(Scope::ALL.iter().map(|s| s.to_string()).collect()),
        Some(list) if list.is_empty() => Err(ApiError::validation("scopes must not be empty")),
        Some(list) => list
            .iter()
            .map(|s| {
                s.parse::<Scope>()
                    .map(|scope| scope.to_string())
                    .map_err(|_| ApiError::validation(format!("unknown scope {s}")))
            })
            .collect(),
    }
}

async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let req: NewProject = json::parse(&body)?;
    let name = req.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(ApiError::validation("name must be 1 to 100 characters"));
    }
    let project = state
        .db
        .create_project(name, state.config.network.as_str())
        .await?;
    let key = apikey::generate(key_label_for(&state.config.network));
    let scopes = parse_scopes(None)?;
    let record = state
        .db
        .create_api_key(&project.id, "default", &scopes, &key)
        .await?;
    let body = json!({
        "project": { "object": "project", "id": project.id, "name": project.name,
                     "network": project.network, "created_at": project.created_at },
        "api_key": {
            "id": record.id,
            "secret": key.secret,
            "scopes": record.scopes,
            "note": "The secret is shown once and cannot be retrieved again.",
        },
    });
    Ok((StatusCode::CREATED, axum::Json(body)).into_response())
}

async fn create_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let req: NewKey = json::parse_or_default(&body)?;
    state.db.get_project(&project_id).await?;
    let scopes = parse_scopes(req.scopes)?;
    let label = req.label.unwrap_or_else(|| "api key".into());
    let key = apikey::generate(key_label_for(&state.config.network));
    let record = state
        .db
        .create_api_key(&project_id, &label, &scopes, &key)
        .await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(json!({
            "object": "api_key", "id": record.id, "label": record.label,
            "prefix": record.prefix, "scopes": record.scopes,
            "secret": key.secret, "created_at": record.created_at,
        })),
    )
        .into_response())
}

async fn list_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    let keys = state.db.list_api_keys(&project_id).await?;
    let data: Vec<_> = keys
        .iter()
        .map(|k| json!({ "object": "api_key", "id": k.id, "label": k.label, "prefix": k.prefix,
                         "scopes": k.scopes, "revoked_at": k.revoked_at, "created_at": k.created_at }))
        .collect();
    Ok(json::ok(
        json!({ "object": "list", "data": data, "has_more": false, "next_cursor": null }),
    ))
}

async fn revoke_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, key_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    state.db.revoke_api_key(&project_id, &key_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn require_dev(state: &AppState) -> Result<(), ApiError> {
    if state.config.enable_dev_endpoints
        && state.config.network == safrod_core::network::Network::Regtest
    {
        Ok(())
    } else {
        Err(ApiError::not_found("endpoint"))
    }
}

#[derive(Deserialize)]
struct MineRequest {
    blocks: u32,
}

/// Regtest convenience: mines to the node's own wallet so a fresh environment
/// can be funded without touching `bitcoin-cli`.
async fn mine(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    require_dev(&state)?;
    let req: MineRequest = json::parse(&body)?;
    if req.blocks == 0 || req.blocks > 200 {
        return Err(ApiError::validation("blocks must be between 1 and 200"));
    }
    let rpc = state
        .rpc
        .clone()
        .ok_or_else(|| ApiError::unavailable("bitcoin core is not configured"))?;
    let address = blocking(&state.ln, |ln| ln.new_onchain_address()).await??;
    let hashes = rpc
        .generate_to_address(req.blocks, &address)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "mining failed");
            ApiError::unavailable("bitcoin core rejected the request")
        })?;
    Ok(json::ok(
        json!({ "mined": hashes.len(), "address": address }),
    ))
}

async fn dev_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers)?;
    require_dev(&state)?;
    let height = match &state.rpc {
        Some(rpc) => rpc.block_count().await.ok(),
        None => None,
    };
    Ok(json::ok(
        json!({ "network": state.config.network.as_str(), "block_height": height }),
    ))
}
