use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use safrod_lightning::LightningError;
use safrod_storage::StorageError;
use serde_json::json;
use tracing::error;

tokio::task_local! {
    pub static REQUEST_ID: String;
}

pub fn current_request_id() -> String {
    REQUEST_ID
        .try_with(|id| id.clone())
        .unwrap_or_else(|_| "req_unknown".to_string())
}

/// Client-facing error. `message` is written for API consumers; internal
/// detail (SQL, node internals) goes to the log, never into this body.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            message,
        )
    }

    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid API key",
        )
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{what} not found"),
        )
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            message,
        )
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal error; quote the request id when reporting it",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": self.message,
                "request_id": current_request_id(),
            }
        });
        (self.status, Json(body)).into_response()
    }
}

impl From<StorageError> for ApiError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::NotFound => ApiError::not_found("resource"),
            StorageError::Conflict => ApiError::conflict("conflict", "resource already exists"),
            StorageError::IdempotencyMismatch => ApiError::conflict(
                "idempotency_key_reused",
                "this Idempotency-Key was already used with different parameters",
            ),
            StorageError::IllegalTransition { from, to } => {
                ApiError::conflict("invalid_state", format!("cannot move from {from} to {to}"))
            }
            StorageError::InsufficientBalance => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "insufficient_simulated_balance",
                "the simulated balance is too low for this payout",
            ),
            StorageError::AmountOutOfRange => ApiError::validation("amount is out of range"),
            other => {
                error!(error = %other, "storage failure");
                ApiError::internal()
            }
        }
    }
}

impl From<LightningError> for ApiError {
    fn from(err: LightningError) -> Self {
        match err {
            LightningError::InvalidInvoice(_) => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_invoice",
                "the invoice could not be parsed",
            ),
            LightningError::NetworkMismatch { expected, found } => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "network_mismatch",
                format!("invoice is for {found} but this node runs on {expected}"),
            ),
            LightningError::InvoiceExpired => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invoice_expired",
                "the invoice has expired",
            ),
            LightningError::AmountRequired => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "amount_required",
                "the invoice has no amount; supply amount_msat",
            ),
            LightningError::DuplicatePayment => ApiError::conflict(
                "duplicate_payment",
                "this invoice is already being paid or was already paid",
            ),
            LightningError::NoRoute => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "no_route",
                "insufficient outbound liquidity or no route to the destination",
            ),
            LightningError::NotRunning => {
                ApiError::unavailable("the lightning node is not running")
            }
            LightningError::Node(detail) => {
                error!(%detail, "lightning node error");
                ApiError::unavailable("the lightning node reported an error")
            }
        }
    }
}
