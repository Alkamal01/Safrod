//! Webhook signatures: `Safrod-Signature: t=<unix seconds>,v1=<hex>` where the
//! MAC is HMAC-SHA256 over `"{t}.{body}"`. Binding the timestamp into the MAC
//! lets receivers reject replays outside a tolerance window.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub const SIGNATURE_HEADER: &str = "Safrod-Signature";
pub const DEFAULT_TOLERANCE_SECS: i64 = 300;

#[derive(Debug, PartialEq, Eq)]
pub enum VerifyError {
    Malformed,
    Mismatch,
    Stale,
}

pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    format!(
        "t={timestamp},v1={}",
        hex::encode(mac(secret, timestamp, body))
    )
}

fn mac(secret: &[u8], timestamp: i64, body: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.finalize().into_bytes().to_vec()
}

pub fn verify(
    secret: &[u8],
    header: &str,
    body: &[u8],
    now: i64,
    tolerance_secs: i64,
) -> Result<(), VerifyError> {
    let mut timestamp = None;
    let mut candidates = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => timestamp = v.parse::<i64>().ok(),
            Some(("v1", v)) => {
                if let Ok(bytes) = hex::decode(v) {
                    candidates.push(bytes);
                }
            }
            _ => {}
        }
    }
    let timestamp = timestamp.ok_or(VerifyError::Malformed)?;
    if candidates.is_empty() {
        return Err(VerifyError::Malformed);
    }
    let expected = mac(secret, timestamp, body);
    // Check the MAC before the clock so a forger learns nothing from the
    // difference in error type.
    let ok = candidates.iter().fold(false, |acc, c| {
        acc | bool::from(c.as_slice().ct_eq(&expected))
    });
    if !ok {
        return Err(VerifyError::Mismatch);
    }
    if (now - timestamp).abs() > tolerance_secs {
        return Err(VerifyError::Stale);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"whsec_test";

    #[test]
    fn valid_signature_verifies() {
        let header = sign(SECRET, 1_000, b"{\"a\":1}");
        assert_eq!(verify(SECRET, &header, b"{\"a\":1}", 1_010, 300), Ok(()));
    }

    #[test]
    fn tampered_body_or_secret_is_rejected() {
        let header = sign(SECRET, 1_000, b"{\"a\":1}");
        assert_eq!(
            verify(SECRET, &header, b"{\"a\":2}", 1_000, 300),
            Err(VerifyError::Mismatch)
        );
        assert_eq!(
            verify(b"other", &header, b"{\"a\":1}", 1_000, 300),
            Err(VerifyError::Mismatch)
        );
    }

    #[test]
    fn stale_timestamp_is_rejected_even_with_valid_mac() {
        let header = sign(SECRET, 1_000, b"x");
        assert_eq!(
            verify(SECRET, &header, b"x", 2_000, 300),
            Err(VerifyError::Stale)
        );
    }

    #[test]
    fn malformed_headers_are_rejected() {
        assert_eq!(
            verify(SECRET, "", b"x", 0, 300),
            Err(VerifyError::Malformed)
        );
        assert_eq!(
            verify(SECRET, "t=1", b"x", 0, 300),
            Err(VerifyError::Malformed)
        );
        assert_eq!(
            verify(SECRET, "v1=zz,t=1", b"x", 0, 300),
            Err(VerifyError::Malformed)
        );
    }
}
