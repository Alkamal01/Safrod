use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub network: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ApiKeyRecord {
    pub id: String,
    pub project_id: String,
    pub label: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub project_id: String,
    pub key_id: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct InvoiceRecord {
    pub id: String,
    pub project_id: String,
    pub idempotency_key: Option<String>,
    pub amount_msat: i64,
    pub description: String,
    pub bolt11: String,
    pub payment_hash: String,
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
    pub amount_received_msat: Option<i64>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct PaymentRecord {
    pub id: String,
    pub project_id: String,
    pub idempotency_key: String,
    pub bolt11: String,
    pub payment_hash: String,
    pub amount_msat: i64,
    pub fee_msat: Option<i64>,
    pub status: String,
    pub failure_reason: Option<String>,
    pub preimage: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AddressRecord {
    pub id: String,
    pub project_id: String,
    pub address: String,
    pub label: String,
    pub required_confirmations: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct OnchainTransactionRecord {
    pub id: String,
    pub project_id: String,
    pub address_id: String,
    pub txid: String,
    pub vout: i32,
    pub amount_sat: i64,
    pub block_height: Option<i32>,
    pub confirmations: i32,
    pub status: String,
    pub first_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct EventRecord {
    pub seq: i64,
    pub id: String,
    pub project_id: String,
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub kind: String,
    pub resource_id: String,
    pub data: serde_json::Value,
    pub simulated: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct WebhookEndpoint {
    pub id: String,
    pub project_id: String,
    pub url: String,
    #[serde(skip)]
    pub secret: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct DeliveryRecord {
    pub id: String,
    pub event_seq: i64,
    pub endpoint_id: String,
    pub status: String,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub last_status: Option<i32>,
    pub last_error: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct DueDelivery {
    pub id: String,
    pub attempts: i32,
    pub url: String,
    pub secret: String,
    pub event_id: String,
    pub event_type: String,
    pub resource_id: String,
    pub data: serde_json::Value,
    pub simulated: bool,
    pub event_created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct QuoteRecord {
    pub id: String,
    pub project_id: String,
    pub source_currency: String,
    pub target_currency: String,
    pub source_minor: i64,
    pub target_minor: i64,
    pub rate_ppm: i64,
    pub btc_amount_msat: i64,
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct PayoutRecord {
    pub id: String,
    pub project_id: String,
    pub quote_id: String,
    pub idempotency_key: String,
    pub scenario: String,
    pub settlement_bolt11: Option<String>,
    pub status: String,
    pub payment_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
