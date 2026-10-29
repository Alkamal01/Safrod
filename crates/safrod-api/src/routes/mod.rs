pub mod account;
pub mod admin;
pub mod bitcoin;
pub mod docs;
pub mod health;
pub mod lightning;
pub mod sandbox;
pub mod webhooks;

use axum::extract::Query;
use axum::extract::rejection::QueryRejection;
use serde::Deserialize;

use crate::error::ApiError;

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub starting_after: Option<String>,
}

pub fn query<T>(result: Result<Query<T>, QueryRejection>) -> Result<T, ApiError> {
    result
        .map(|Query(q)| q)
        .map_err(|_| ApiError::validation("invalid query parameters"))
}

/// Resource ids that fail to parse can't exist, so they are reported as
/// not found rather than as a validation error.
pub fn id<T: std::str::FromStr>(raw: &str, what: &str) -> Result<T, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found(what))
}
