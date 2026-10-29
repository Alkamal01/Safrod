//! Bitcoin amounts as integers. Satoshis and millisatoshis are distinct types
//! with no implicit conversion so a unit mixup is visible at the call site.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

pub const MSAT_PER_SAT: u64 = 1_000;

/// Upper bound on any single amount accepted from a client: 21M BTC in sats.
/// Rejecting above this keeps msat conversion far from `u64` overflow.
pub const MAX_SATS: u64 = 21_000_000 * 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Satoshis(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Millisatoshis(pub u64);

impl Satoshis {
    pub const ZERO: Satoshis = Satoshis(0);

    /// Validates an amount received over the API: non-zero and within supply.
    pub fn from_request(sats: u64) -> Result<Self, CoreError> {
        if sats == 0 {
            return Err(CoreError::InvalidAmount("amount must be greater than zero"));
        }
        if sats > MAX_SATS {
            return Err(CoreError::InvalidAmount(
                "amount exceeds the bitcoin supply",
            ));
        }
        Ok(Satoshis(sats))
    }

    pub fn checked_add(self, other: Satoshis) -> Option<Satoshis> {
        self.0.checked_add(other.0).map(Satoshis)
    }

    pub fn checked_sub(self, other: Satoshis) -> Option<Satoshis> {
        self.0.checked_sub(other.0).map(Satoshis)
    }

    pub fn to_msat(self) -> Option<Millisatoshis> {
        self.0.checked_mul(MSAT_PER_SAT).map(Millisatoshis)
    }
}

impl Millisatoshis {
    pub const ZERO: Millisatoshis = Millisatoshis(0);

    pub fn checked_add(self, other: Millisatoshis) -> Option<Millisatoshis> {
        self.0.checked_add(other.0).map(Millisatoshis)
    }

    pub fn checked_sub(self, other: Millisatoshis) -> Option<Millisatoshis> {
        self.0.checked_sub(other.0).map(Millisatoshis)
    }

    /// Rounds down. A sub-satoshi remainder cannot be settled on-chain, so
    /// callers that need exactness must check `0 % 1000` themselves.
    pub fn to_sats_floor(self) -> Satoshis {
        Satoshis(self.0 / MSAT_PER_SAT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_and_oversupply() {
        assert!(Satoshis::from_request(0).is_err());
        assert!(Satoshis::from_request(MAX_SATS + 1).is_err());
        assert_eq!(Satoshis::from_request(1).unwrap(), Satoshis(1));
    }

    #[test]
    fn msat_conversion_is_checked() {
        assert_eq!(Satoshis(2).to_msat(), Some(Millisatoshis(2_000)));
        assert_eq!(Satoshis(u64::MAX).to_msat(), None);
        assert_eq!(Millisatoshis(1_999).to_sats_floor(), Satoshis(1));
    }

    #[test]
    fn arithmetic_does_not_wrap() {
        assert_eq!(Satoshis(u64::MAX).checked_add(Satoshis(1)), None);
        assert_eq!(Satoshis::ZERO.checked_sub(Satoshis(1)), None);
        assert_eq!(Millisatoshis(u64::MAX).checked_add(Millisatoshis(1)), None);
    }
}
