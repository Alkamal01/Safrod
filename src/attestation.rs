//! Signed fiat payout attestations.
//!
//! A signature proves who signed a statement. It does not by itself prove
//! that the statement corresponds to a real-world fiat event — that's a
//! bounded-trust assumption the rest of the node has to account for
//! (disputes, exposure limits), not something this module can fix.

use bitcoin::hashes::{Hash, HashEngine, sha256};
use bitcoin::secp256k1::{self, Message, PublicKey, Secp256k1, SecretKey, ecdsa::Signature};
use serde::{Deserialize, Serialize};

use crate::ids::SettlementId;
use crate::state::PayoutOutcome;

/// A signed claim from one provider's node that a fiat payout reached (or
/// definitely did not reach) its beneficiary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutAttestation {
    pub settlement_id: SettlementId,
    pub status: PayoutOutcome,
    /// Opaque reference into the fiat rail's own records (e.g. its
    /// transaction id), so the claim can be checked against the rail later.
    pub evidence_ref: String,
    pub observed_at_unix: u64,
    /// Compressed secp256k1 public key (33 bytes) of the signer.
    pub signer_pubkey: Vec<u8>,
    /// Compact ECDSA signature (64 bytes).
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttestationError {
    #[error("attestation signer public key or signature is malformed")]
    Malformed,
    #[error("attestation signature does not verify against its claimed signer")]
    InvalidSignature,
}

fn digest(
    settlement_id: SettlementId,
    status: PayoutOutcome,
    evidence_ref: &str,
    observed_at_unix: u64,
) -> [u8; 32] {
    let mut engine = sha256::Hash::engine();
    // Domain-separate this digest from any other signed message the same
    // key might produce.
    engine.input(b"safro/payout-attestation/v1");
    engine.input(settlement_id.as_bytes());
    engine.input(&[status as u8]);
    engine.input(&(evidence_ref.len() as u64).to_be_bytes());
    engine.input(evidence_ref.as_bytes());
    engine.input(&observed_at_unix.to_be_bytes());
    sha256::Hash::from_engine(engine).to_byte_array()
}

pub fn sign(
    secp: &Secp256k1<secp256k1::All>,
    signing_key: &SecretKey,
    settlement_id: SettlementId,
    status: PayoutOutcome,
    evidence_ref: impl Into<String>,
    observed_at_unix: u64,
) -> PayoutAttestation {
    let evidence_ref = evidence_ref.into();
    let message = Message::from_digest(digest(
        settlement_id,
        status,
        &evidence_ref,
        observed_at_unix,
    ));
    let signature = secp.sign_ecdsa(&message, signing_key);
    let signer_pubkey = PublicKey::from_secret_key(secp, signing_key);

    PayoutAttestation {
        settlement_id,
        status,
        evidence_ref,
        observed_at_unix,
        signer_pubkey: signer_pubkey.serialize().to_vec(),
        signature: signature.serialize_compact().to_vec(),
    }
}

/// Verifies the attestation's signature against its own claimed signer.
/// This only proves the named key produced this exact statement — the
/// caller is responsible for deciding whether that signer is trusted for
/// this settlement.
pub fn verify(
    secp: &Secp256k1<secp256k1::All>,
    attestation: &PayoutAttestation,
) -> Result<(), AttestationError> {
    let pubkey = PublicKey::from_slice(&attestation.signer_pubkey)
        .map_err(|_| AttestationError::Malformed)?;
    let signature =
        Signature::from_compact(&attestation.signature).map_err(|_| AttestationError::Malformed)?;
    let message = Message::from_digest(digest(
        attestation.settlement_id,
        attestation.status,
        &attestation.evidence_ref,
        attestation.observed_at_unix,
    ));

    secp.verify_ecdsa(&message, &signature, &pubkey)
        .map_err(|_| AttestationError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key(byte: u8) -> SecretKey {
        SecretKey::from_slice(&[byte; 32]).expect("valid test key")
    }

    #[test]
    fn valid_attestation_verifies() {
        let secp = Secp256k1::new();
        let key = test_key(7);
        let attestation = sign(
            &secp,
            &key,
            SettlementId::new(),
            PayoutOutcome::Confirmed,
            "rail-tx-123",
            1_700_000_000,
        );

        assert_eq!(verify(&secp, &attestation), Ok(()));
    }

    #[test]
    fn tampered_evidence_fails_verification() {
        let secp = Secp256k1::new();
        let key = test_key(7);
        let mut attestation = sign(
            &secp,
            &key,
            SettlementId::new(),
            PayoutOutcome::Confirmed,
            "rail-tx-123",
            1_700_000_000,
        );

        attestation.evidence_ref = "rail-tx-999".to_string();

        assert_eq!(
            verify(&secp, &attestation),
            Err(AttestationError::InvalidSignature)
        );
    }

    #[test]
    fn signed_by_a_different_key_fails_verification() {
        let secp = Secp256k1::new();
        let signer_key = test_key(7);
        let mut attestation = sign(
            &secp,
            &signer_key,
            SettlementId::new(),
            PayoutOutcome::Confirmed,
            "rail-tx-123",
            1_700_000_000,
        );

        let other_key = test_key(9);
        let other_pubkey = PublicKey::from_secret_key(&secp, &other_key);
        attestation.signer_pubkey = other_pubkey.serialize().to_vec();

        assert_eq!(
            verify(&secp, &attestation),
            Err(AttestationError::InvalidSignature)
        );
    }

    #[test]
    fn malformed_signature_bytes_are_rejected() {
        let secp = Secp256k1::new();
        let key = test_key(7);
        let mut attestation = sign(
            &secp,
            &key,
            SettlementId::new(),
            PayoutOutcome::Confirmed,
            "rail-tx-123",
            1_700_000_000,
        );

        attestation.signature = vec![0u8; 3];

        assert_eq!(
            verify(&secp, &attestation),
            Err(AttestationError::Malformed)
        );
    }
}
