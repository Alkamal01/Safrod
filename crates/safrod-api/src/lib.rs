pub mod auth;
pub mod engine;
pub mod error;
pub mod json;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use safrod_bitcoin::BitcoinRpc;
use safrod_core::network::Network;
use safrod_lightning::LightningBackend;
use safrod_storage::Db;
use tracing::info;

use crate::error::REQUEST_ID;

pub struct ApiConfig {
    pub network: Network,
    /// `None` disables the admin API entirely.
    pub admin_token: Option<String>,
    pub webhook_allow_private: bool,
    pub default_confirmations: i32,
    pub enable_dev_endpoints: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub ln: Arc<dyn LightningBackend>,
    pub rpc: Option<BitcoinRpc>,
    pub config: Arc<ApiConfig>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(routes::health::routes())
        .merge(routes::docs::routes())
        .merge(routes::admin::routes())
        .merge(routes::lightning::routes())
        .merge(routes::bitcoin::routes())
        .merge(routes::account::routes())
        .merge(routes::webhooks::routes())
        .merge(routes::sandbox::routes())
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn(request_context))
        .with_state(state)
}

/// Assigns a request id, exposes it to error bodies, logs the request, and
/// sets conservative response headers.
async fn request_context(request: Request, next: Next) -> Response {
    let id = format!("req_{}", uuid::Uuid::new_v4().simple());
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let mut response = REQUEST_ID.scope(id.clone(), next.run(request)).await;
    info!(request_id = %id, %method, %path, status = response.status().as_u16(), "request");
    let headers = response.headers_mut();
    headers.insert(
        "x-request-id",
        HeaderValue::from_str(&id).expect("request id is ascii"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
