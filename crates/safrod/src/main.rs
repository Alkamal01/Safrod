mod config;
mod tasks;

use std::sync::Arc;

use safrod_api::{ApiConfig, AppState, router};
use safrod_bitcoin::{BitcoinRpc, Watcher};
use safrod_lightning::LightningBackend;
use safrod_lightning::ldk::{LdkBackend, LdkConfig};
use safrod_lightning::mock::MockBackend;
use safrod_storage::Db;
use safrod_webhooks::{DeliveryConfig, Worker};
use tokio::sync::watch;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::{Config, LightningKind};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    if let Err(e) = run().await {
        error!("{e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    info!(network = config.network.as_str(), bind = %config.bind, "starting safrod");
    if config.admin_token.is_none() {
        warn!("SAFROD_ADMIN_TOKEN is not set: the admin API is disabled");
    }

    let db = Db::connect(&config.database_url).await?;
    db.migrate().await?;

    let rpc = match &config.bitcoin_rpc_url {
        Some(url) => Some(BitcoinRpc::new(
            url,
            &config.bitcoin_rpc_user,
            &config.bitcoin_rpc_password,
        )?),
        None => None,
    };

    let (ln, ldk): (Arc<dyn LightningBackend>, Option<Arc<LdkBackend>>) = match config.lightning {
        LightningKind::Mock => {
            warn!("using the mock Lightning backend: no real payments");
            (Arc::new(MockBackend::new()), None)
        }
        LightningKind::Ldk => {
            let (rpc_host, rpc_port) = config.rpc_host_port()?;
            create_private_dir(&config.ldk_storage_dir)?;
            let ldk_config = LdkConfig {
                network: config.network,
                storage_dir: config.ldk_storage_dir.clone(),
                listen: config.ldk_listen.clone(),
                alias: config.ldk_alias.clone(),
                rpc_host,
                rpc_port,
                rpc_user: config.bitcoin_rpc_user.clone(),
                rpc_password: config.bitcoin_rpc_password.clone(),
            };
            let backend =
                tokio::task::spawn_blocking(move || LdkBackend::start(ldk_config)).await??;
            info!(node_id = %backend.info().node_id, "lightning node started");
            (backend.clone() as Arc<dyn LightningBackend>, Some(backend))
        }
    };

    let state = AppState {
        db: db.clone(),
        ln,
        rpc: rpc.clone(),
        config: Arc::new(ApiConfig {
            network: config.network,
            admin_token: config.admin_token.clone(),
            webhook_allow_private: config.webhook_allow_private,
            default_confirmations: config.required_confirmations,
            enable_dev_endpoints: config.enable_dev_endpoints,
        }),
    };

    // Payments left in flight by a previous run are compared with the node
    // before any new traffic is accepted.
    match safrod_api::engine::reconcile_all(&state).await {
        Ok(n) => info!(changed = n, "startup reconciliation finished"),
        Err(e) => warn!(error = ?e.message, "startup reconciliation failed"),
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let mut handles = Vec::new();
    if let Some(ldk) = ldk.clone() {
        handles.push(tokio::spawn(tasks::pump_node_events(
            ldk,
            state.clone(),
            stop_rx.clone(),
        )));
    }
    handles.push(tokio::spawn(tasks::expire_invoices(
        db.clone(),
        stop_rx.clone(),
    )));
    handles.push(tokio::spawn(
        Worker::new(
            db.clone(),
            DeliveryConfig {
                allow_private: config.webhook_allow_private,
                ..Default::default()
            },
        )
        .run(stop_rx.clone()),
    ));
    if let Some(rpc) = rpc {
        handles.push(tokio::spawn(
            Watcher::new(rpc, db.clone()).run(config.watch_interval, stop_rx.clone()),
        ));
    }

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    info!(addr = %config.bind, "api listening");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(tasks::shutdown_signal())
        .await?;

    info!("shutting down");
    let _ = stop_tx.send(true);
    for handle in handles {
        let _ = handle.await;
    }
    if let Some(ldk) = ldk {
        let _ = tokio::task::spawn_blocking(move || ldk.stop()).await;
    }
    Ok(())
}

/// The LDK data directory holds the node's wallet seed, so it is created
/// readable by the owner only.
fn create_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}
