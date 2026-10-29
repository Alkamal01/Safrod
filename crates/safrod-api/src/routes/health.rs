use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use safrod_core::network::Network;
use serde_json::json;

use crate::AppState;
use crate::engine::blocking;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

fn core_chain_name(network: Network) -> &'static str {
    match network {
        Network::Regtest => "regtest",
        Network::Signet => "signet",
        Network::Testnet => "test",
        Network::Bitcoin => "main",
    }
}

async fn readyz(State(state): State<AppState>) -> Response {
    let database = state.db.ping().await.is_ok();
    let info = blocking(&state.ln, |ln| ln.info()).await.ok();
    let node = info.as_ref().map(|i| i.running).unwrap_or(false);
    let bitcoin = match &state.rpc {
        Some(rpc) => rpc
            .chain()
            .await
            .map(|c| c == core_chain_name(state.config.network))
            .unwrap_or(false),
        None => true,
    };
    let ready = database && node && bitcoin;
    let body = json!({
        "status": if ready { "ready" } else { "not_ready" },
        "network": state.config.network.as_str(),
        "checks": { "database": database, "lightning_node": node, "bitcoin_core": bitcoin },
    });
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body)).into_response()
}
