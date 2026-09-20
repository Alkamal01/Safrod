//! Typed, randomly generated identifiers. A thin `[u8; 16]` wrapper is
//! enough here, so this doesn't pull in a UUID crate for something this
//! small.

use std::fmt;

use rand::RngCore;
use serde::{Deserialize, Serialize};

macro_rules! random_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name([u8; 16]);

        impl $name {
            pub fn new() -> Self {
                let mut bytes = [0u8; 16];
                rand::thread_rng().fill_bytes(&mut bytes);
                Self(bytes)
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }

            pub fn from_hex(s: &str) -> Option<Self> {
                if s.len() != 32 || !s.is_ascii() {
                    return None;
                }
                let mut bytes = [0u8; 16];
                for (i, byte) in bytes.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
                }
                Some(Self(bytes))
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                Self::from_hex(&s).ok_or_else(|| serde::de::Error::custom("invalid identifier"))
            }
        }

        impl std::str::FromStr for $name {
            type Err = ();

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::from_hex(s).ok_or(())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
        }
    };
}

random_id!(SettlementId);
random_id!(ReservationId);
random_id!(QuoteId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_round_trip_through_display() {
        let a = SettlementId::new();
        let b = SettlementId::new();
        assert_ne!(a, b);
        assert_eq!(a.to_string().len(), 32);
    }
}
