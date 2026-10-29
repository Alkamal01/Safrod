use axum::http::HeaderMap;
use safrod_core::apikey::{self, Scope};
use safrod_storage::models::AuthContext;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::error::ApiError;

pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    let value = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::unauthorized)?;
    let token = value
        .strip_prefix("Bearer ")
        .ok_or_else(ApiError::unauthorized)?
        .trim();
    apikey::looks_like_key(token).map_err(|_| ApiError::unauthorized())?;
    state
        .db
        .authenticate(&apikey::hash_secret(token))
        .await?
        .ok_or_else(ApiError::unauthorized)
}

pub async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    scope: Scope,
) -> Result<AuthContext, ApiError> {
    let ctx = authenticate(state, headers).await?;
    if ctx.scopes.iter().any(|s| s == scope.as_str()) {
        Ok(ctx)
    } else {
        Err(ApiError::forbidden(format!(
            "this API key lacks the {scope} scope"
        )))
    }
}

/// Operator-only endpoints use a separate static token. It is compared via
/// digests so the comparison is constant-time regardless of length.
pub fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = state.config.admin_token.as_deref() else {
        return Err(ApiError::forbidden(
            "the admin API is disabled on this node",
        ));
    };
    let provided = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::forbidden("admin token required"))?;
    let a = Sha256::digest(expected.as_bytes());
    let b = Sha256::digest(provided.as_bytes());
    if bool::from(a.ct_eq(&b)) {
        Ok(())
    } else {
        Err(ApiError::forbidden("admin token required"))
    }
}
