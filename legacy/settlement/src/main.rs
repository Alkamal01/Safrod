use std::path::PathBuf;
use std::sync::Arc;

use safrod::adapters::simulated::SimulatedFiatAdapter;
use safrod::api::{self, AppState};
use safrod::config::NodeConfig;
use safrod::lightning::cln::ClnLightningSettlement;
use safrod::node::Node;
use safrod::store::FileSettlementStore;

#[tokio::main]
async fn main() {
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));

    let config = NodeConfig::load(&config_path).unwrap_or_else(|e| {
        eprintln!("failed to load config from {}: {e}", config_path.display());
        std::process::exit(1);
    });

    let store = FileSettlementStore::open(&config.settlement_store_path).unwrap_or_else(|e| {
        eprintln!("failed to open settlement store: {e}");
        std::process::exit(1);
    });
    let node = Arc::new(
        Node::new(
            SimulatedFiatAdapter::new(),
            ClnLightningSettlement::new(config.lightning_rpc_socket.clone()),
            store,
            config.identity_key,
        )
        .with_peer_identity(config.peer_identity),
    );
    node.recover().unwrap_or_else(|e| {
        eprintln!("failed to recover pending settlements: {e}");
        std::process::exit(1);
    });

    let state = AppState {
        node,
        peer_addr: config.peer_addr.clone(),
        http: reqwest::Client::new(),
    };

    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {}: {e}", config.listen_addr));

    println!(
        "safrod listening on {} (currency={:?}, peer={})",
        config.listen_addr, config.currency, config.peer_addr
    );

    axum::serve(listener, api::router(state))
        .await
        .expect("server error");
}
