//! Node configuration, loaded from a small TOML file. Each of the two demo
//! nodes gets its own file (see `configs/`).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use bitcoin::secp256k1::{PublicKey, SecretKey};
use serde::Deserialize;

use crate::money::Currency;

#[derive(Debug, Deserialize)]
struct RawNodeConfig {
    identity_key_hex: String,
    peer_identity_key_hex: String,
    listen_addr: String,
    currency: Currency,
    peer_addr: String,
    /// Path to this node's `lightningd` `lightning-rpc` Unix socket.
    lightning_rpc_socket: String,
    settlement_store_path: String,
}

#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub identity_key: SecretKey,
    pub peer_identity: PublicKey,
    pub listen_addr: SocketAddr,
    pub currency: Currency,
    pub peer_addr: String,
    pub lightning_rpc_socket: PathBuf,
    pub settlement_store_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config file: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("identity_key_hex is not a valid secp256k1 secret key")]
    InvalidKey,
    #[error("peer_identity_key_hex is not a valid secp256k1 public key")]
    InvalidPeerKey,
    #[error("listen_addr is not a valid socket address: {0}")]
    InvalidAddr(String),
}

impl NodeConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let raw: RawNodeConfig = toml::from_str(&contents)?;

        let identity_key =
            SecretKey::from_str(&raw.identity_key_hex).map_err(|_| ConfigError::InvalidKey)?;
        let peer_identity = PublicKey::from_str(&raw.peer_identity_key_hex)
            .map_err(|_| ConfigError::InvalidPeerKey)?;
        let listen_addr = raw
            .listen_addr
            .parse()
            .map_err(|_| ConfigError::InvalidAddr(raw.listen_addr.clone()))?;

        Ok(Self {
            identity_key,
            peer_identity,
            listen_addr,
            currency: raw.currency,
            peer_addr: raw.peer_addr,
            lightning_rpc_socket: PathBuf::from(raw.lightning_rpc_socket),
            settlement_store_path: PathBuf::from(raw.settlement_store_path),
        })
    }
}
