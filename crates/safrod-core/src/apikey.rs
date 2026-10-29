//! API keys. Only the SHA-256 digest of a key is stored. Keys are 256 random
//! bits so a plain hash (rather than a password KDF) is sufficient: there is
//! nothing to brute-force.

use std::fmt;
use std::str::FromStr;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::CoreError;

const KEY_PREFIX: &str = "sfd";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "invoices:read")]
    InvoicesRead,
    #[serde(rename = "invoices:write")]
    InvoicesWrite,
    #[serde(rename = "payments:read")]
    PaymentsRead,
    #[serde(rename = "payments:write")]
    PaymentsWrite,
    #[serde(rename = "bitcoin:read")]
    BitcoinRead,
    #[serde(rename = "bitcoin:write")]
    BitcoinWrite,
    #[serde(rename = "events:read")]
    EventsRead,
    #[serde(rename = "webhooks:write")]
    WebhooksWrite,
    #[serde(rename = "sandbox:write")]
    SandboxWrite,
}

impl Scope {
    pub const ALL: [Scope; 9] = [
        Scope::InvoicesRead,
        Scope::InvoicesWrite,
        Scope::PaymentsRead,
        Scope::PaymentsWrite,
        Scope::BitcoinRead,
        Scope::BitcoinWrite,
        Scope::EventsRead,
        Scope::WebhooksWrite,
        Scope::SandboxWrite,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::InvoicesRead => "invoices:read",
            Scope::InvoicesWrite => "invoices:write",
            Scope::PaymentsRead => "payments:read",
            Scope::PaymentsWrite => "payments:write",
            Scope::BitcoinRead => "bitcoin:read",
            Scope::BitcoinWrite => "bitcoin:write",
            Scope::EventsRead => "events:read",
            Scope::WebhooksWrite => "webhooks:write",
            Scope::SandboxWrite => "sandbox:write",
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Scope {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Scope::ALL
            .into_iter()
            .find(|scope| scope.as_str() == s)
            .ok_or_else(|| CoreError::UnknownScope(s.to_string()))
    }
}

pub struct GeneratedKey {
    /// Shown to the caller exactly once.
    pub secret: String,
    pub prefix: String,
    pub hash: Vec<u8>,
}

pub fn generate(label: &str) -> GeneratedKey {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let secret = format!("{KEY_PREFIX}_{label}_{}", hex::encode(bytes));
    GeneratedKey {
        prefix: secret[..secret.len().min(KEY_PREFIX.len() + label.len() + 9)].to_string(),
        hash: hash_secret(&secret),
        secret,
    }
}

pub fn hash_secret(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

/// Cheap shape check before touching the database.
pub fn looks_like_key(candidate: &str) -> Result<(), CoreError> {
    if candidate.starts_with("sfd_") && candidate.len() <= 128 && candidate.is_ascii() {
        Ok(())
    } else {
        Err(CoreError::InvalidApiKey)
    }
}

pub fn digest_matches(stored: &[u8], candidate_secret: &str) -> bool {
    stored.ct_eq(&hash_secret(candidate_secret)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_verifies_and_hides_secret_in_prefix() {
        let key = generate("test");
        assert!(key.secret.starts_with("sfd_test_"));
        assert!(digest_matches(&key.hash, &key.secret));
        assert!(!digest_matches(&key.hash, "sfd_test_wrong"));
        assert!(key.prefix.len() < key.secret.len());
        assert!(looks_like_key(&key.secret).is_ok());
        assert!(looks_like_key("Bearer x").is_err());
    }

    #[test]
    fn scopes_round_trip() {
        for scope in Scope::ALL {
            assert_eq!(scope.as_str().parse::<Scope>().unwrap(), scope);
        }
        assert!("admin:all".parse::<Scope>().is_err());
    }
}
