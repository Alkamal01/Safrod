//! Prefixed identifiers (`inv_…`, `pay_…`) so a payment id can never be
//! mistaken for an invoice id in logs or API calls.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CoreError;

macro_rules! prefixed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, Uuid::new_v4().simple()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let rest = s
                    .strip_prefix(concat!($prefix, "_"))
                    .ok_or(CoreError::InvalidId)?;
                if rest.len() == 32 && rest.bytes().all(|b| b.is_ascii_hexdigit()) {
                    Ok(Self(s.to_ascii_lowercase()))
                } else {
                    Err(CoreError::InvalidId)
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

prefixed_id!(ProjectId, "prj");
prefixed_id!(ApiKeyId, "key");
prefixed_id!(InvoiceId, "inv");
prefixed_id!(PaymentId, "pay");
prefixed_id!(AddressId, "adr");
prefixed_id!(TransactionId, "txn");
prefixed_id!(EventId, "evt");
prefixed_id!(WebhookEndpointId, "whe");
prefixed_id!(DeliveryId, "dlv");
prefixed_id!(QuoteId, "qte");
prefixed_id!(PayoutId, "pot");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_wrong_prefix() {
        let inv = InvoiceId::new();
        assert_eq!(inv.to_string().parse::<InvoiceId>().unwrap(), inv);
        assert!(inv.to_string().parse::<PaymentId>().is_err());
        assert!("inv_xyz".parse::<InvoiceId>().is_err());
        assert!("inv_".parse::<InvoiceId>().is_err());
    }

    #[test]
    fn serde_uses_the_string_form() {
        let id = PaymentId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<PaymentId>(&json).unwrap(), id);
    }
}
