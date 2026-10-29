//! Payment engine: the code that decides what a node event or an API request
//! means for durable state. Handlers here must be safe to run twice, because
//! LDK Node redelivers unacknowledged events after a restart.

use std::sync::Arc;

use safrod_core::ids::PaymentId;
use safrod_core::state::PaymentStatus;
use safrod_lightning::{LightningBackend, LightningError, NodeEvent, NodePaymentStatus};
use safrod_storage::StorageError;
use safrod_storage::models::PaymentRecord;
use safrod_storage::payments::{NewPayment, Resolution};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::AppState;
use crate::error::ApiError;

/// LDK Node calls can block on its internal runtime, so they never run on an
/// async worker thread.
pub async fn blocking<T: Send + 'static>(
    ln: &Arc<dyn LightningBackend>,
    f: impl FnOnce(&dyn LightningBackend) -> T + Send + 'static,
) -> Result<T, ApiError> {
    let ln = ln.clone();
    tokio::task::spawn_blocking(move || f(ln.as_ref()))
        .await
        .map_err(|_| ApiError::internal())
}

pub struct SendRequest {
    pub bolt11: String,
    pub amount_msat: Option<u64>,
    pub metadata: Value,
}

pub struct SendResult {
    pub payment: PaymentRecord,
    pub created: bool,
}

pub async fn send_payment(
    state: &AppState,
    project_id: &str,
    idempotency_key: &str,
    request: SendRequest,
) -> Result<SendResult, ApiError> {
    let bolt11 = request.bolt11.trim().to_string();
    let parse_target = bolt11.clone();
    let parsed = blocking(&state.ln, move |ln| ln.parse_invoice(&parse_target)).await??;
    let amount_msat = match (parsed.amount_msat, request.amount_msat) {
        (Some(invoice), Some(given)) if invoice != given => {
            return Err(ApiError::validation(
                "amount_msat does not match the amount encoded in the invoice",
            ));
        }
        (Some(invoice), _) => invoice,
        (None, Some(given)) => given,
        (None, None) => return Err(LightningError::AmountRequired.into()),
    };
    let sats = safrod_core::money::Millisatoshis(amount_msat).to_sats_floor();
    if sats.0 == 0 || sats.0 > safrod_core::money::MAX_SATS {
        return Err(ApiError::validation(
            "amount must be between one satoshi and the bitcoin supply",
        ));
    }

    let mut hasher = Sha256::new();
    hasher.update(bolt11.as_bytes());
    hasher.update(amount_msat.to_be_bytes());
    hasher.update(request.metadata.to_string().as_bytes());
    let begun = state
        .db
        .begin_payment(NewPayment {
            id: PaymentId::new(),
            project_id: project_id.to_string(),
            idempotency_key: idempotency_key.to_string(),
            request_hash: hasher.finalize().to_vec(),
            bolt11: bolt11.clone(),
            payment_hash: parsed.payment_hash.clone(),
            amount_msat,
            metadata: request.metadata,
        })
        .await?;
    if !begun.created {
        // A retry of a request we already accepted. Dispatching again could
        // pay twice, so return what we have.
        return Ok(SendResult {
            payment: begun.payment,
            created: false,
        });
    }

    // `pending` is durable before the node is asked to send. If the process
    // dies after this point the payment is reconciled, never silently retried.
    let pending = state
        .db
        .transition_payment(
            None,
            &begun.payment.id,
            PaymentStatus::Pending,
            Resolution::default(),
        )
        .await?;
    let amount_for_node = parsed.amount_msat.is_none().then_some(amount_msat);
    let dispatch = bolt11.clone();
    let sent = blocking(&state.ln, move |ln| {
        ln.send_payment(&dispatch, amount_for_node)
    })
    .await?;
    match sent {
        Ok(_) => Ok(SendResult {
            payment: pending.payment,
            created: true,
        }),
        Err(err) => {
            // The node refused before an HTLC left, so the failure is definite.
            let failed = state
                .db
                .transition_payment(
                    None,
                    &begun.payment.id,
                    PaymentStatus::Failed,
                    Resolution {
                        failure_reason: Some(failure_label(&err).to_string()),
                        ..Default::default()
                    },
                )
                .await?;
            Ok(SendResult {
                payment: failed.payment,
                created: true,
            })
        }
    }
}

fn failure_label(err: &LightningError) -> &'static str {
    match err {
        LightningError::NoRoute => "no_route",
        LightningError::DuplicatePayment => "duplicate_payment",
        LightningError::NotRunning => "node_not_running",
        _ => "node_rejected",
    }
}

/// Compares a non-final payment with the node's own record.
///
/// If the node has no record of a payment that was already `pending`, the send
/// may or may not have happened, so it becomes `unknown` rather than `failed`.
pub async fn reconcile_payment(
    state: &AppState,
    payment: &PaymentRecord,
) -> Result<PaymentRecord, ApiError> {
    let current: PaymentStatus = payment.status.parse().map_err(|_| ApiError::internal())?;
    if current.is_final() {
        return Ok(payment.clone());
    }
    let hash = payment.payment_hash.clone();
    let node_view = blocking(&state.ln, move |ln| ln.payment(&hash)).await?;
    let (next, resolution) = match node_view {
        Some(p) if p.status == NodePaymentStatus::Succeeded => (
            PaymentStatus::Succeeded,
            Resolution {
                fee_msat: p.fee_msat,
                preimage: p.preimage,
                ..Default::default()
            },
        ),
        Some(p) if p.status == NodePaymentStatus::Failed => (
            PaymentStatus::Failed,
            Resolution {
                failure_reason: Some("reported_failed_by_node".into()),
                ..Default::default()
            },
        ),
        Some(_) => return Ok(payment.clone()),
        None if current == PaymentStatus::Created => (
            PaymentStatus::Failed,
            Resolution {
                failure_reason: Some("never_dispatched".into()),
                ..Default::default()
            },
        ),
        None if current == PaymentStatus::Unknown => return Ok(payment.clone()),
        None => (PaymentStatus::Unknown, Resolution::default()),
    };
    // `Created` cannot move straight to `unknown`/`succeeded`; pass through pending.
    if current == PaymentStatus::Created && next != PaymentStatus::Failed {
        state
            .db
            .transition_payment(
                None,
                &payment.id,
                PaymentStatus::Pending,
                Resolution::default(),
            )
            .await?;
    }
    Ok(state
        .db
        .transition_payment(None, &payment.id, next, resolution)
        .await?
        .payment)
}

pub async fn reconcile_all(state: &AppState) -> Result<usize, ApiError> {
    let open = state.db.unresolved_payments().await?;
    let mut changed = 0;
    for payment in open {
        let after = reconcile_payment(state, &payment).await?;
        if after.status != payment.status {
            changed += 1;
            info!(payment = %payment.id, from = %payment.status, to = %after.status, "reconciled payment");
        }
    }
    Ok(changed)
}

#[derive(Debug)]
pub enum EventError {
    /// Retrying will not help (for example an unknown invoice). The event
    /// should be acknowledged and logged.
    Permanent(String),
    /// A database or infrastructure failure. Retry the same event.
    Transient(String),
}

fn classify(err: StorageError) -> EventError {
    match err {
        StorageError::NotFound | StorageError::IllegalTransition { .. } => {
            EventError::Permanent(err.to_string())
        }
        other => EventError::Transient(other.to_string()),
    }
}

pub async fn handle_node_event(state: &AppState, event: NodeEvent) -> Result<(), EventError> {
    match event {
        NodeEvent::PaymentReceived {
            payment_hash,
            amount_msat,
        } => {
            let key = format!("recv:{payment_hash}");
            match state
                .db
                .mark_invoice_paid(&key, &payment_hash, amount_msat)
                .await
            {
                Ok(Some(invoice)) => info!(invoice = %invoice.id, amount_msat, "invoice paid"),
                Ok(None) => info!(%payment_hash, "payment event already applied"),
                Err(e) => return Err(classify(e)),
            }
        }
        NodeEvent::PaymentSuccessful {
            payment_hash,
            preimage,
            fee_msat,
        } => {
            let Some(payment) = unresolved(state, &payment_hash).await? else {
                warn!(%payment_hash, "success event for a payment with no open record");
                return Ok(());
            };
            state
                .db
                .transition_payment(
                    Some(&format!("sent_ok:{}", payment.id)),
                    &payment.id,
                    PaymentStatus::Succeeded,
                    Resolution {
                        fee_msat,
                        preimage,
                        ..Default::default()
                    },
                )
                .await
                .map_err(classify)?;
        }
        NodeEvent::PaymentFailed {
            payment_hash,
            reason,
        } => {
            let Some(hash) = payment_hash else {
                warn!(%reason, "payment failed before an invoice was resolved");
                return Ok(());
            };
            let Some(payment) = unresolved(state, &hash).await? else {
                warn!(payment_hash = %hash, "failure event for a payment with no open record");
                return Ok(());
            };
            state
                .db
                .transition_payment(
                    Some(&format!("sent_fail:{}", payment.id)),
                    &payment.id,
                    PaymentStatus::Failed,
                    Resolution {
                        failure_reason: Some(reason),
                        ..Default::default()
                    },
                )
                .await
                .map_err(classify)?;
        }
        NodeEvent::ChannelReady { channel_id } => info!(%channel_id, "channel ready"),
        NodeEvent::ChannelClosed { channel_id, reason } => {
            warn!(%channel_id, %reason, "channel closed")
        }
        NodeEvent::Other(kind) => info!(%kind, "ignored node event"),
    }
    Ok(())
}

async fn unresolved(state: &AppState, hash: &str) -> Result<Option<PaymentRecord>, EventError> {
    state
        .db
        .find_unresolved_payment_by_hash(hash)
        .await
        .map_err(classify)
}
