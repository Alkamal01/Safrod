//! Payment state machines. Every status change in the database goes through
//! `can_transition_to`, so an impossible jump (for example a failed payment
//! becoming succeeded) is a bug that surfaces as an error instead of silently
//! rewriting history.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

macro_rules! status_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok($name::$variant),)+
                    _ => Err(CoreError::InvalidId),
                }
            }
        }
    };
}

status_enum!(InvoiceStatus {
    Open => "open",
    Paid => "paid",
    Expired => "expired",
});

impl InvoiceStatus {
    pub fn is_final(self) -> bool {
        !matches!(self, InvoiceStatus::Open)
    }

    pub fn can_transition_to(self, next: InvoiceStatus) -> bool {
        matches!(
            (self, next),
            (InvoiceStatus::Open, InvoiceStatus::Paid)
                | (InvoiceStatus::Open, InvoiceStatus::Expired)
        )
    }
}

status_enum!(PaymentStatus {
    Created => "created",
    Pending => "pending",
    Succeeded => "succeeded",
    Failed => "failed",
    Unknown => "unknown",
});

impl PaymentStatus {
    pub fn is_final(self) -> bool {
        matches!(self, PaymentStatus::Succeeded | PaymentStatus::Failed)
    }

    /// `Unknown` means the outcome could not be determined (restart while the
    /// HTLC was in flight, node unreachable). It is deliberately not a
    /// failure: retrying blindly could pay the same invoice twice. It can only
    /// be resolved by a definitive node event or reconciliation.
    pub fn can_transition_to(self, next: PaymentStatus) -> bool {
        use PaymentStatus::*;
        matches!(
            (self, next),
            (Created, Pending)
                | (Created, Failed)
                | (Pending, Succeeded)
                | (Pending, Failed)
                | (Pending, Unknown)
                | (Unknown, Succeeded)
                | (Unknown, Failed)
        )
    }

    pub fn transition(self, next: PaymentStatus) -> Result<PaymentStatus, CoreError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(CoreError::IllegalTransition {
                from: self.as_str(),
                to: next.as_str(),
            })
        }
    }
}

status_enum!(OnchainStatus {
    Awaiting => "awaiting",
    Unconfirmed => "unconfirmed",
    Confirming => "confirming",
    Confirmed => "confirmed",
});

impl OnchainStatus {
    pub fn from_confirmations(confirmations: u32, required: u32) -> OnchainStatus {
        match confirmations {
            0 => OnchainStatus::Unconfirmed,
            n if n < required => OnchainStatus::Confirming,
            _ => OnchainStatus::Confirmed,
        }
    }
}

status_enum!(DeliveryStatus {
    Pending => "pending",
    Delivered => "delivered",
    Failed => "failed",
});

status_enum!(QuoteStatus {
    Open => "open",
    Used => "used",
    Expired => "expired",
});

status_enum!(PayoutStatus {
    Reserved => "reserved",
    Pending => "pending",
    Succeeded => "succeeded",
    Failed => "failed",
    Unknown => "unknown",
});

impl PayoutStatus {
    pub fn can_transition_to(self, next: PayoutStatus) -> bool {
        use PayoutStatus::*;
        matches!(
            (self, next),
            (Reserved, Pending)
                | (Pending, Succeeded)
                | (Pending, Failed)
                | (Pending, Unknown)
                | (Unknown, Succeeded)
                | (Unknown, Failed)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_payment_states_never_move() {
        for next in [
            PaymentStatus::Created,
            PaymentStatus::Pending,
            PaymentStatus::Succeeded,
            PaymentStatus::Failed,
            PaymentStatus::Unknown,
        ] {
            assert!(!PaymentStatus::Succeeded.can_transition_to(next));
            assert!(!PaymentStatus::Failed.can_transition_to(next));
        }
    }

    #[test]
    fn unknown_resolves_only_to_a_definitive_outcome() {
        assert!(PaymentStatus::Unknown.can_transition_to(PaymentStatus::Succeeded));
        assert!(PaymentStatus::Unknown.can_transition_to(PaymentStatus::Failed));
        assert!(!PaymentStatus::Unknown.can_transition_to(PaymentStatus::Pending));
        assert!(!PaymentStatus::Unknown.can_transition_to(PaymentStatus::Created));
    }

    #[test]
    fn invoice_only_leaves_open() {
        assert!(InvoiceStatus::Open.can_transition_to(InvoiceStatus::Paid));
        assert!(!InvoiceStatus::Paid.can_transition_to(InvoiceStatus::Expired));
        assert!(!InvoiceStatus::Expired.can_transition_to(InvoiceStatus::Paid));
    }

    #[test]
    fn onchain_status_follows_confirmations() {
        assert_eq!(
            OnchainStatus::from_confirmations(0, 3),
            OnchainStatus::Unconfirmed
        );
        assert_eq!(
            OnchainStatus::from_confirmations(2, 3),
            OnchainStatus::Confirming
        );
        assert_eq!(
            OnchainStatus::from_confirmations(3, 3),
            OnchainStatus::Confirmed
        );
    }

    #[test]
    fn statuses_round_trip_through_text() {
        let s = PaymentStatus::Unknown;
        assert_eq!(s.as_str().parse::<PaymentStatus>().unwrap(), s);
        assert!("bogus".parse::<PaymentStatus>().is_err());
    }
}
