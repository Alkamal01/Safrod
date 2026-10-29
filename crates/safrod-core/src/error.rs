use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("invalid amount: {0}")]
    InvalidAmount(&'static str),
    #[error("unknown network: {0}")]
    UnknownNetwork(String),
    #[error("{what} is for {found}, but this node runs on {expected}")]
    NetworkMismatch {
        what: &'static str,
        expected: &'static str,
        found: &'static str,
    },
    #[error("illegal state transition {from} -> {to}")]
    IllegalTransition {
        from: &'static str,
        to: &'static str,
    },
    #[error("invalid identifier")]
    InvalidId,
    #[error("invalid api key")]
    InvalidApiKey,
    #[error("unknown scope: {0}")]
    UnknownScope(String),
}
