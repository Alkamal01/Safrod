use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Regtest,
    Signet,
    Testnet,
    Bitcoin,
}

impl Network {
    pub fn as_str(self) -> &'static str {
        match self {
            Network::Regtest => "regtest",
            Network::Signet => "signet",
            Network::Testnet => "testnet",
            Network::Bitcoin => "bitcoin",
        }
    }

    /// Safrod is a prototype and refuses to start on mainnet unless a future
    /// release deliberately lifts this.
    pub fn is_supported(self) -> bool {
        !matches!(self, Network::Bitcoin)
    }

    /// BOLT11 human-readable prefix, e.g. `lnbcrt` on regtest.
    pub fn bolt11_prefix(self) -> &'static str {
        match self {
            Network::Regtest => "lnbcrt",
            Network::Signet => "lntbs",
            Network::Testnet => "lntb",
            Network::Bitcoin => "lnbc",
        }
    }

    /// Checks an invoice string's prefix against this network. This is a
    /// cheap pre-check; the Lightning library still fully parses the invoice.
    pub fn check_invoice_prefix(self, invoice: &str) -> Result<(), CoreError> {
        let lower = invoice.trim().to_ascii_lowercase();
        let found = [
            Network::Regtest,
            Network::Signet,
            Network::Testnet,
            Network::Bitcoin,
        ]
        .into_iter()
        // `lnbcrt` must be tried before `lnbc`, and `lntbs` before `lntb`.
        .find(|n| {
            let p = n.bolt11_prefix();
            lower.starts_with(p)
                && lower[p.len()..].starts_with(|c: char| c.is_ascii_digit() || c == '1')
        });
        match found {
            Some(n) if n == self => Ok(()),
            Some(n) => Err(CoreError::NetworkMismatch {
                what: "invoice",
                expected: self.as_str(),
                found: n.as_str(),
            }),
            None => Err(CoreError::NetworkMismatch {
                what: "invoice",
                expected: self.as_str(),
                found: "unknown",
            }),
        }
    }
}

impl FromStr for Network {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "regtest" => Ok(Network::Regtest),
            "signet" => Ok(Network::Signet),
            "testnet" => Ok(Network::Testnet),
            "bitcoin" | "mainnet" => Ok(Network::Bitcoin),
            other => Err(CoreError::UnknownNetwork(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_check_distinguishes_overlapping_prefixes() {
        assert!(
            Network::Regtest
                .check_invoice_prefix("lnbcrt1u1pabc")
                .is_ok()
        );
        assert!(
            Network::Bitcoin
                .check_invoice_prefix("lnbc10u1pabc")
                .is_ok()
        );
        assert!(
            Network::Testnet
                .check_invoice_prefix("lntb20m1pabc")
                .is_ok()
        );
        assert!(Network::Signet.check_invoice_prefix("lntbs1pabc").is_ok());
        assert!(
            Network::Regtest
                .check_invoice_prefix("lnbc10u1pabc")
                .is_err()
        );
        assert!(Network::Regtest.check_invoice_prefix("garbage").is_err());
    }

    #[test]
    fn mainnet_is_not_supported() {
        assert!(!Network::Bitcoin.is_supported());
        assert!(Network::Regtest.is_supported());
    }
}
