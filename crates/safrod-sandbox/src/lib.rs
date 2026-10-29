//! Everything in this crate is simulated. Rates are fixed constants, not
//! market data, and no fiat ever moves. Responses built from it are labelled
//! `simulated` by the API so they can't be mistaken for settled money.

use std::fmt;
use std::str::FromStr;

use safrod_core::money::MAX_SATS;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fixed BTC price used for every simulated quote.
pub const SIMULATED_BTC_USD: u64 = 60_000;
pub const SIMULATED_FEE_BPS: u64 = 100;
const MSAT_PER_BTC: u128 = 100_000_000_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SandboxError {
    #[error("unsupported currency: {0}")]
    UnsupportedCurrency(String),
    #[error("source and target currency must differ")]
    SameCurrency,
    #[error("amount must be greater than zero and within range")]
    BadAmount,
    #[error("unknown scenario: {0}")]
    UnknownScenario(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Currency {
    Usd,
    Eur,
    Ngn,
    Kes,
}

impl Currency {
    pub fn code(self) -> &'static str {
        match self {
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
            Currency::Ngn => "NGN",
            Currency::Kes => "KES",
        }
    }

    /// Minor units of this currency per one US dollar (simulated).
    fn minor_per_usd(self) -> u128 {
        match self {
            Currency::Usd => 100,
            Currency::Eur => 92,
            Currency::Ngn => 150_000,
            Currency::Kes => 12_900,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Currency {
    type Err = SandboxError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "USD" => Ok(Currency::Usd),
            "EUR" => Ok(Currency::Eur),
            "NGN" => Ok(Currency::Ngn),
            "KES" => Ok(Currency::Kes),
            other => Err(SandboxError::UnsupportedCurrency(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteCalc {
    pub target_minor: u64,
    /// Target minor units per source minor unit, in parts per million.
    pub rate_ppm: u64,
    pub btc_amount_msat: u64,
}

/// Prices a cross-currency transfer with integer math only. The fee is taken
/// from the target amount; the BTC leg is sized from the source amount.
pub fn quote(
    source: Currency,
    target: Currency,
    source_minor: u64,
) -> Result<QuoteCalc, SandboxError> {
    if source == target {
        return Err(SandboxError::SameCurrency);
    }
    if source_minor == 0 {
        return Err(SandboxError::BadAmount);
    }
    let src = u128::from(source_minor);
    let gross_target = src * target.minor_per_usd() / source.minor_per_usd();
    let target_minor = gross_target * u128::from(10_000 - SIMULATED_FEE_BPS) / 10_000;
    let rate_ppm = target.minor_per_usd() * 1_000_000 / source.minor_per_usd();
    let btc_msat = src * MSAT_PER_BTC / (source.minor_per_usd() * u128::from(SIMULATED_BTC_USD));

    let to_u64 = |v: u128| u64::try_from(v).map_err(|_| SandboxError::BadAmount);
    let btc_amount_msat = to_u64(btc_msat)?;
    let target_minor = to_u64(target_minor)?;
    if target_minor == 0 || btc_amount_msat == 0 || btc_amount_msat > MAX_SATS * 1_000 {
        return Err(SandboxError::BadAmount);
    }
    Ok(QuoteCalc {
        target_minor,
        rate_ppm: to_u64(rate_ppm)?,
        btc_amount_msat,
    })
}

/// What the simulated destination payout rail will do. `Timeout` models an
/// ambiguous answer: the payout may or may not have happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    Success,
    Failure,
    Timeout,
}

impl Scenario {
    pub fn as_str(self) -> &'static str {
        match self {
            Scenario::Success => "success",
            Scenario::Failure => "failure",
            Scenario::Timeout => "timeout",
        }
    }
}

impl FromStr for Scenario {
    type Err = SandboxError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "success" => Ok(Scenario::Success),
            "failure" => Ok(Scenario::Failure),
            "timeout" => Ok(Scenario::Timeout),
            other => Err(SandboxError::UnknownScenario(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_is_deterministic_and_integer_only() {
        let q = quote(Currency::Ngn, Currency::Kes, 500_000).unwrap();
        // 5,000.00 NGN * (12_900 / 150_000) = 430.00 KES gross, less 1% fee.
        assert_eq!(q.target_minor, 42_570);
        assert_eq!(q.rate_ppm, 86_000);
        assert_eq!(quote(Currency::Ngn, Currency::Kes, 500_000).unwrap(), q);
    }

    #[test]
    fn btc_leg_is_sized_from_the_source_amount() {
        // $60 at $60,000/BTC is 0.001 BTC = 100,000,000 msat.
        let q = quote(Currency::Usd, Currency::Eur, 6_000).unwrap();
        assert_eq!(q.btc_amount_msat, 100_000_000);
    }

    #[test]
    fn rejects_bad_requests() {
        assert_eq!(
            quote(Currency::Usd, Currency::Usd, 100),
            Err(SandboxError::SameCurrency)
        );
        assert_eq!(
            quote(Currency::Usd, Currency::Eur, 0),
            Err(SandboxError::BadAmount)
        );
        // Dust that rounds to nothing on the BTC leg.
        assert_eq!(
            quote(Currency::Ngn, Currency::Usd, 1),
            Err(SandboxError::BadAmount)
        );
        assert!(quote(Currency::Usd, Currency::Eur, u64::MAX).is_err());
    }

    #[test]
    fn parses_currencies_and_scenarios() {
        assert_eq!("ngn".parse::<Currency>().unwrap(), Currency::Ngn);
        assert!("btc".parse::<Currency>().is_err());
        assert_eq!("timeout".parse::<Scenario>().unwrap(), Scenario::Timeout);
        assert!("explode".parse::<Scenario>().is_err());
    }
}
