//! JSON helpers that keep every client-visible error in the standard shape.

use axum::Json;
use axum::body::Bytes;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use safrod_storage::models::{
    AddressRecord, DeliveryRecord, EventRecord, InvoiceRecord, OnchainTransactionRecord,
    PaymentRecord, PayoutRecord, QuoteRecord, WebhookEndpoint,
};

use crate::error::ApiError;

pub fn parse<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|e| ApiError::validation(format!("invalid request body: {e}")))
}

pub fn parse_or_default<T: DeserializeOwned + Default>(body: &Bytes) -> Result<T, ApiError> {
    if body.is_empty() {
        return Ok(T::default());
    }
    parse(body)
}

pub fn idempotency_key(headers: &HeaderMap, required: bool) -> Result<Option<String>, ApiError> {
    let Some(value) = headers.get("idempotency-key") else {
        return if required {
            Err(ApiError::bad_request(
                "idempotency_key_required",
                "an Idempotency-Key header is required",
            ))
        } else {
            Ok(None)
        };
    };
    let key = value
        .to_str()
        .map_err(|_| {
            ApiError::bad_request("invalid_idempotency_key", "Idempotency-Key must be ascii")
        })?
        .trim();
    if key.is_empty() || key.len() > 255 {
        return Err(ApiError::bad_request(
            "invalid_idempotency_key",
            "Idempotency-Key must be 1 to 255 characters",
        ));
    }
    Ok(Some(key.to_string()))
}

pub fn created(created: bool, body: Value) -> Response {
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    (status, Json(body)).into_response()
}

pub fn ok(body: Value) -> Response {
    (StatusCode::OK, Json(body)).into_response()
}

pub struct Page {
    pub limit: i64,
}

impl Page {
    pub fn from_query(limit: Option<i64>) -> Result<Self, ApiError> {
        match limit {
            None => Ok(Page { limit: 20 }),
            Some(n) if (1..=100).contains(&n) => Ok(Page { limit: n }),
            Some(_) => Err(ApiError::validation("limit must be between 1 and 100")),
        }
    }

    /// Fetch one extra row so `has_more` is exact.
    pub fn fetch(&self) -> i64 {
        self.limit + 1
    }

    pub fn wrap<T>(
        &self,
        mut rows: Vec<T>,
        to_json: impl Fn(&T) -> Value,
        id_of: impl Fn(&T) -> String,
    ) -> Value {
        let has_more = rows.len() as i64 > self.limit;
        rows.truncate(self.limit as usize);
        let next_cursor = if has_more {
            rows.last().map(&id_of)
        } else {
            None
        };
        json!({
            "object": "list",
            "data": rows.iter().map(to_json).collect::<Vec<_>>(),
            "has_more": has_more,
            "next_cursor": next_cursor,
        })
    }
}

pub fn invoice(r: &InvoiceRecord) -> Value {
    json!({
        "object": "lightning_invoice",
        "id": r.id,
        "status": r.status,
        "amount_msat": r.amount_msat,
        "description": r.description,
        "bolt11": r.bolt11,
        "payment_hash": r.payment_hash,
        "expires_at": r.expires_at,
        "paid_at": r.paid_at,
        "amount_received_msat": r.amount_received_msat,
        "metadata": r.metadata,
        "created_at": r.created_at,
        "real_regtest_lightning": true,
        "simulated": false,
    })
}

pub fn payment(r: &PaymentRecord, reveal_preimage: bool) -> Value {
    json!({
        "object": "lightning_payment",
        "id": r.id,
        "status": r.status,
        "amount_msat": r.amount_msat,
        "fee_msat": r.fee_msat,
        "bolt11": r.bolt11,
        "payment_hash": r.payment_hash,
        // Proof of payment. Only shown on regtest, where it is not a secret
        // with any value; on a real network it would be withheld from logs.
        "preimage": if reveal_preimage { r.preimage.clone() } else { None },
        "failure_reason": r.failure_reason,
        "metadata": r.metadata,
        "created_at": r.created_at,
        "updated_at": r.updated_at,
        "real_regtest_lightning": true,
        "simulated": false,
    })
}

pub fn address(r: &AddressRecord) -> Value {
    json!({
        "object": "bitcoin_address",
        "id": r.id,
        "address": r.address,
        "label": r.label,
        "required_confirmations": r.required_confirmations,
        "created_at": r.created_at,
    })
}

pub fn onchain_tx(r: &OnchainTransactionRecord, address: &str) -> Value {
    json!({
        "object": "bitcoin_transaction",
        "id": r.id,
        "address_id": r.address_id,
        "address": address,
        "txid": r.txid,
        "vout": r.vout,
        "amount_sat": r.amount_sat,
        "confirmations": r.confirmations,
        "block_height": r.block_height,
        "status": r.status,
        "first_seen_at": r.first_seen_at,
        "updated_at": r.updated_at,
        "simulated": false,
    })
}

pub fn event(r: &EventRecord) -> Value {
    json!({
        "object": "event",
        "id": r.id,
        "seq": r.seq,
        "type": r.kind,
        "resource_id": r.resource_id,
        "data": r.data,
        "simulated": r.simulated,
        "created_at": r.created_at,
    })
}

pub fn endpoint(r: &WebhookEndpoint) -> Value {
    json!({
        "object": "webhook_endpoint",
        "id": r.id,
        "url": r.url,
        "enabled": r.enabled,
        "created_at": r.created_at,
    })
}

pub fn delivery(r: &DeliveryRecord) -> Value {
    json!({
        "object": "webhook_delivery",
        "id": r.id,
        "event_seq": r.event_seq,
        "endpoint_id": r.endpoint_id,
        "status": r.status,
        "attempts": r.attempts,
        "next_attempt_at": r.next_attempt_at,
        "last_status": r.last_status,
        "last_error": r.last_error,
        "delivered_at": r.delivered_at,
        "created_at": r.created_at,
    })
}

pub fn quote(r: &QuoteRecord) -> Value {
    json!({
        "object": "sandbox_quote",
        "id": r.id,
        "status": r.status,
        "source": { "currency": r.source_currency, "minor_units": r.source_minor },
        "target": { "currency": r.target_currency, "minor_units": r.target_minor },
        "rate_ppm": r.rate_ppm,
        "btc_amount_msat": r.btc_amount_msat,
        "expires_at": r.expires_at,
        "created_at": r.created_at,
        "simulated_fiat": true,
        "real_regtest_lightning": false,
    })
}

pub fn payout(r: &PayoutRecord) -> Value {
    json!({
        "object": "sandbox_payout",
        "id": r.id,
        "quote_id": r.quote_id,
        "status": r.status,
        "scenario": r.scenario,
        "payment_id": r.payment_id,
        "created_at": r.created_at,
        "updated_at": r.updated_at,
        "simulated_fiat": true,
        "real_regtest_lightning": r.payment_id.is_some(),
    })
}
