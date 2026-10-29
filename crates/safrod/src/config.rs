use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use safrod_core::network::Network;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{name} is invalid: {reason}")]
    Invalid { name: &'static str, reason: String },
    #[error("refusing to start: {0}")]
    Unsafe(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightningKind {
    Ldk,
    /// In-memory stand-in for UI and API development. Moves no funds.
    Mock,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub network: Network,
    pub bind: SocketAddr,
    pub database_url: String,
    pub admin_token: Option<String>,
    pub enable_dev_endpoints: bool,
    pub webhook_allow_private: bool,
    pub required_confirmations: i32,
    pub watch_interval: Duration,
    pub lightning: LightningKind,
    pub bitcoin_rpc_url: Option<String>,
    pub bitcoin_rpc_user: String,
    pub bitcoin_rpc_password: String,
    pub ldk_storage_dir: PathBuf,
    pub ldk_listen: String,
    pub ldk_alias: String,
}

fn parse<T: FromStr>(
    get: &dyn Fn(&str) -> Option<String>,
    name: &'static str,
    default: &str,
) -> Result<T, ConfigError>
where
    T::Err: std::fmt::Display,
{
    get(name)
        .unwrap_or_else(|| default.to_string())
        .parse()
        .map_err(|e: T::Err| ConfigError::Invalid {
            name,
            reason: e.to_string(),
        })
}

fn flag(get: &dyn Fn(&str) -> Option<String>, name: &'static str) -> Result<bool, ConfigError> {
    match get(name).as_deref() {
        None | Some("") | Some("false") | Some("0") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(other) => Err(ConfigError::Invalid {
            name,
            reason: format!("expected true or false, got {other}"),
        }),
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::load(&|name| std::env::var(name).ok())
    }

    pub fn load(get: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let network: Network = parse(get, "SAFROD_NETWORK", "regtest").map_err(|e| match e {
            ConfigError::Invalid { reason, .. } => ConfigError::Invalid {
                name: "SAFROD_NETWORK",
                reason,
            },
            other => other,
        })?;
        if !network.is_supported() {
            return Err(ConfigError::Unsafe(
                "mainnet is not supported: Safrod is an unaudited prototype",
            ));
        }
        let bind: SocketAddr = parse(get, "SAFROD_BIND", "127.0.0.1:8080")?;
        if !bind.ip().is_loopback() && !flag(get, "SAFROD_ALLOW_PUBLIC_BIND")? {
            return Err(ConfigError::Unsafe(
                "SAFROD_BIND is not a loopback address; set SAFROD_ALLOW_PUBLIC_BIND=true only behind a firewall or a published-to-localhost container port",
            ));
        }
        let database_url = get("SAFROD_DATABASE_URL").ok_or(ConfigError::Missing("SAFROD_DATABASE_URL"))?;

        let admin_token = get("SAFROD_ADMIN_TOKEN").filter(|t| !t.is_empty());
        if let Some(token) = &admin_token
            && token.len() < 24
        {
            return Err(ConfigError::Invalid {
                name: "SAFROD_ADMIN_TOKEN",
                reason: "must be at least 24 characters".into(),
            });
        }
        let enable_dev_endpoints = flag(get, "SAFROD_ENABLE_DEV_ENDPOINTS")?;
        if enable_dev_endpoints && network != Network::Regtest {
            return Err(ConfigError::Unsafe("dev endpoints are only allowed on regtest"));
        }

        let lightning = match get("SAFROD_LIGHTNING_BACKEND").as_deref() {
            None | Some("ldk") => LightningKind::Ldk,
            Some("mock") => LightningKind::Mock,
            Some(other) => {
                return Err(ConfigError::Invalid {
                    name: "SAFROD_LIGHTNING_BACKEND",
                    reason: format!("expected ldk or mock, got {other}"),
                });
            }
        };
        let required_confirmations: i32 = parse(get, "SAFROD_REQUIRED_CONFIRMATIONS", "3")?;
        if !(1..=100).contains(&required_confirmations) {
            return Err(ConfigError::Invalid {
                name: "SAFROD_REQUIRED_CONFIRMATIONS",
                reason: "must be between 1 and 100".into(),
            });
        }
        let watch_secs: u64 = parse(get, "SAFROD_WATCH_INTERVAL_SECS", "2")?;

        let config = Config {
            network,
            bind,
            database_url,
            admin_token,
            enable_dev_endpoints,
            webhook_allow_private: flag(get, "SAFROD_WEBHOOK_ALLOW_PRIVATE")?,
            required_confirmations,
            watch_interval: Duration::from_secs(watch_secs.max(1)),
            lightning,
            bitcoin_rpc_url: get("SAFROD_BITCOIN_RPC_URL").filter(|u| !u.is_empty()),
            bitcoin_rpc_user: get("SAFROD_BITCOIN_RPC_USER").unwrap_or_default(),
            bitcoin_rpc_password: get("SAFROD_BITCOIN_RPC_PASSWORD").unwrap_or_default(),
            ldk_storage_dir: PathBuf::from(
                get("SAFROD_LDK_STORAGE_DIR").unwrap_or_else(|| "./data/ldk".into()),
            ),
            ldk_listen: get("SAFROD_LDK_LISTEN").unwrap_or_else(|| "0.0.0.0:9735".into()),
            ldk_alias: get("SAFROD_LDK_ALIAS").unwrap_or_else(|| "safrod".into()),
        };
        if config.lightning == LightningKind::Ldk && config.bitcoin_rpc_url.is_none() {
            return Err(ConfigError::Missing("SAFROD_BITCOIN_RPC_URL"));
        }
        if config.lightning == LightningKind::Mock && network != Network::Regtest {
            return Err(ConfigError::Unsafe("the mock Lightning backend is regtest-only"));
        }
        Ok(config)
    }

    /// Host and port of the Bitcoin Core RPC endpoint, as LDK Node wants them.
    pub fn rpc_host_port(&self) -> Result<(String, u16), ConfigError> {
        let url = self
            .bitcoin_rpc_url
            .as_deref()
            .ok_or(ConfigError::Missing("SAFROD_BITCOIN_RPC_URL"))?;
        let parsed = reqwest::Url::parse(url).map_err(|e| ConfigError::Invalid {
            name: "SAFROD_BITCOIN_RPC_URL",
            reason: e.to_string(),
        })?;
        let host = parsed.host_str().ok_or(ConfigError::Invalid {
            name: "SAFROD_BITCOIN_RPC_URL",
            reason: "missing host".into(),
        })?;
        Ok((host.to_string(), parsed.port().unwrap_or(18443)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn load(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::load(&|name| map.get(name).cloned())
    }

    const BASE: [(&str, &str); 2] = [
        ("SAFROD_DATABASE_URL", "postgres://x"),
        ("SAFROD_BITCOIN_RPC_URL", "http://127.0.0.1:18443"),
    ];

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        BASE.iter().chain(extra).copied().collect()
    }

    #[test]
    fn defaults_are_safe() {
        let c = load(&BASE).unwrap();
        assert_eq!(c.network, Network::Regtest);
        assert!(c.bind.ip().is_loopback());
        assert!(c.admin_token.is_none());
        assert!(!c.enable_dev_endpoints);
        assert!(!c.webhook_allow_private);
    }

    #[test]
    fn mainnet_is_refused() {
        assert!(matches!(
            load(&with(&[("SAFROD_NETWORK", "bitcoin")])),
            Err(ConfigError::Unsafe(_))
        ));
    }

    #[test]
    fn public_bind_needs_an_explicit_opt_in() {
        assert!(matches!(
            load(&with(&[("SAFROD_BIND", "0.0.0.0:8080")])),
            Err(ConfigError::Unsafe(_))
        ));
        assert!(load(&with(&[("SAFROD_BIND", "0.0.0.0:8080"), ("SAFROD_ALLOW_PUBLIC_BIND", "true")])).is_ok());
    }

    #[test]
    fn short_admin_token_is_rejected() {
        assert!(load(&with(&[("SAFROD_ADMIN_TOKEN", "short")])).is_err());
        assert!(load(&with(&[("SAFROD_ADMIN_TOKEN", "0123456789012345678901234")])).is_ok());
    }

    #[test]
    fn dev_endpoints_and_mock_are_regtest_only() {
        assert!(load(&with(&[("SAFROD_NETWORK", "signet"), ("SAFROD_ENABLE_DEV_ENDPOINTS", "true")])).is_err());
        assert!(load(&with(&[("SAFROD_NETWORK", "signet"), ("SAFROD_LIGHTNING_BACKEND", "mock")])).is_err());
    }

    #[test]
    fn ldk_needs_a_bitcoin_rpc_url() {
        assert!(matches!(
            load(&[("SAFROD_DATABASE_URL", "postgres://x")]),
            Err(ConfigError::Missing("SAFROD_BITCOIN_RPC_URL"))
        ));
        let c = load(&BASE).unwrap();
        assert_eq!(c.rpc_host_port().unwrap(), ("127.0.0.1".to_string(), 18443));
    }

    #[test]
    fn database_url_is_required() {
        assert!(matches!(load(&[]), Err(ConfigError::Missing("SAFROD_DATABASE_URL"))));
    }
}
