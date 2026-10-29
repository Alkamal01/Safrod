use std::sync::Arc;
use std::time::Duration;

use safrod_api::AppState;
use safrod_api::engine::{EventError, handle_node_event};
use safrod_lightning::ldk::LdkBackend;
use safrod_storage::Db;
use tokio::sync::watch;
use tracing::{error, warn};

/// Applies node events one at a time and acknowledges each only after it is
/// durably recorded. An unacknowledged event is redelivered by the node, which
/// is safe because every handler is idempotent.
pub async fn pump_node_events(
    ldk: Arc<LdkBackend>,
    state: AppState,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        let event = tokio::select! {
            event = ldk.next_event() => event,
            _ = stop.changed() => return,
        };
        loop {
            match handle_node_event(&state, event.clone()).await {
                Ok(()) => break,
                Err(EventError::Permanent(reason)) => {
                    error!(?event, %reason, "dropping event that cannot be applied");
                    break;
                }
                Err(EventError::Transient(reason)) => {
                    warn!(?event, %reason, "event handling failed, retrying");
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                        _ = stop.changed() => return,
                    }
                }
            }
        }
        let ack = ldk.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || ack.event_handled()).await {
            error!(error = %e, "event acknowledgement task failed");
        }
    }
}

pub async fn expire_invoices(db: Db, mut stop: watch::Receiver<bool>) {
    loop {
        match db.expire_invoices().await {
            Ok(0) => {}
            Ok(n) => tracing::info!(expired = n, "invoices expired"),
            Err(e) => error!(error = %e, "invoice expiry failed"),
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(15)) => {}
            _ = stop.changed() => return,
        }
    }
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
