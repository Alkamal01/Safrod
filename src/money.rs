//! Monetary types. No floating point anywhere: BTC-denominated amounts are
//! integer satoshis/millisatoshis, fiat amounts are integer minor units tied
//! to a specific currency. Conversions between units are always explicit.

use std::fmt;

use serde::{Deserialize, Serialize};

/// An amount denominated in satoshis (1 BTC = 100_000_000 sats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Satoshis(pub u64);

impl Satoshis {
    pub const ZERO: Satoshis = Satoshis(0);

    pub fn checked_add(self, other: Satoshis) -> Option<Satoshis> {
        self.0.checked_add(other.0).map(Satoshis)
    }

    pub fn checked_sub(self, other: Satoshis) -> Option<Satoshis> {
        self.0.checked_sub(other.0).map(Satoshis)
    }

    /// Explicit widening conversion. There is no implicit `From` so a
    /// sats/msats mixup at a call site is always visible.
    pub fn to_millisatoshis(self) -> Option<Millisatoshis> {
        self.0.checked_mul(1_000).map(Millisatoshis)
    }
}

/// An amount denominated in millisatoshis, the unit Lightning payments and
/// fees are expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Millisatoshis(pub u64);

impl Millisatoshis {
    pub const ZERO: Millisatoshis = Millisatoshis(0);

    pub fn checked_add(self, other: Millisatoshis) -> Option<Millisatoshis> {
        self.0.checked_add(other.0).map(Millisatoshis)
    }

    pub fn checked_sub(self, other: Millisatoshis) -> Option<Millisatoshis> {
        self.0.checked_sub(other.0).map(Millisatoshis)
    }
}

/// Fiat currencies Safro v0.1 knows how to settle. Kept to what the
/// Buildathon corridor needs; adding a currency is a one-line change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Currency {
    Ngn,
    Kes,
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Currency::Ngn => write!(f, "NGN"),
            Currency::Kes => write!(f, "KES"),
        }
    }
}

/// A fiat amount in the currency's minor unit (e.g. kobo for NGN, cents for
/// KES). `minor_units` is signed so a refund/adjustment can be represented
/// without a separate type, but callers constructing a payable amount should
/// treat negative values as a bug, not a valid payout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FiatAmount {
    pub currency: Currency,
    pub minor_units: i64,
}

impl FiatAmount {
    pub fn new(currency: Currency, minor_units: i64) -> Self {
        Self {
            currency,
            minor_units,
        }
    }

    /// Adds two amounts of the same currency. Returns `None` on overflow or
    /// on a currency mismatch — callers must not silently sum NGN and KES.
    pub fn checked_add(self, other: FiatAmount) -> Option<FiatAmount> {
        if self.currency != other.currency {
            return None;
        }
        self.minor_units
            .checked_add(other.minor_units)
            .map(|minor_units| FiatAmount {
                currency: self.currency,
                minor_units,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn satoshis_to_millisatoshis_is_explicit_and_checked() {
        assert_eq!(Satoshis(1).to_millisatoshis(), Some(Millisatoshis(1_000)));
        assert_eq!(Satoshis(u64::MAX).to_millisatoshis(), None);
    }

    #[test]
    fn fiat_amounts_of_different_currencies_do_not_sum() {
        let ngn = FiatAmount::new(Currency::Ngn, 1_000);
        let kes = FiatAmount::new(Currency::Kes, 1_000);
        assert_eq!(ngn.checked_add(kes), None);
    }

    #[test]
    fn satoshi_arithmetic_is_checked() {
        assert_eq!(Satoshis(u64::MAX).checked_add(Satoshis(1)), None);
        assert_eq!(Satoshis::ZERO.checked_sub(Satoshis(1)), None);
    }
}
