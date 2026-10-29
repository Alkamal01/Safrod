//! Executable quotes bind the two providers to the exact obligation before
//! either local rail is touched.

use bitcoin::hashes::{Hash, HashEngine, sha256};
use bitcoin::secp256k1::{self, Message, PublicKey, Secp256k1, SecretKey, ecdsa::Signature};
use serde::{Deserialize, Serialize};

use crate::ids::QuoteId;
use crate::money::{FiatAmount, Millisatoshis};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteTerms {
    pub quote_id: QuoteId,
    pub source_amount: FiatAmount,
    pub destination_amount: FiatAmount,
    pub settlement_amount: Millisatoshis,
    pub fee_amount: FiatAmount,
    pub expires_at_unix: u64,
    pub source_provider: Vec<u8>,
    pub destination_provider: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutableQuote {
    pub terms: QuoteTerms,
    pub signer: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuoteError {
    #[error("quote is expired")]
    Expired,
    #[error("quote signer or signature is malformed")]
    Malformed,
    #[error("quote signature is invalid")]
    InvalidSignature,
    #[error("quote's provider identities do not match this corridor")]
    WrongProviders,
}

fn digest(terms: &QuoteTerms) -> [u8; 32] {
    let bytes = serde_json::to_vec(terms).expect("quote terms serialize");
    let mut engine = sha256::Hash::engine();
    engine.input(b"safro/executable-quote/v1");
    engine.input(&bytes);
    sha256::Hash::from_engine(engine).to_byte_array()
}

pub fn sign(
    secp: &Secp256k1<secp256k1::All>,
    signing_key: &SecretKey,
    terms: QuoteTerms,
) -> ExecutableQuote {
    let signature = secp.sign_ecdsa(&Message::from_digest(digest(&terms)), signing_key);
    let signer = PublicKey::from_secret_key(secp, signing_key)
        .serialize()
        .to_vec();
    ExecutableQuote {
        terms,
        signer,
        signature: signature.serialize_compact().to_vec(),
    }
}

pub fn verify(
    secp: &Secp256k1<secp256k1::All>,
    quote: &ExecutableQuote,
    expected_signer: &PublicKey,
    source_provider: &PublicKey,
    destination_provider: &PublicKey,
    now_unix: u64,
) -> Result<(), QuoteError> {
    if now_unix >= quote.terms.expires_at_unix {
        return Err(QuoteError::Expired);
    }
    let signer = PublicKey::from_slice(&quote.signer).map_err(|_| QuoteError::Malformed)?;
    let signature = Signature::from_compact(&quote.signature).map_err(|_| QuoteError::Malformed)?;
    if &signer != expected_signer
        || quote.terms.source_provider != source_provider.serialize()
        || quote.terms.destination_provider != destination_provider.serialize()
    {
        return Err(QuoteError::WrongProviders);
    }
    secp.verify_ecdsa(
        &Message::from_digest(digest(&quote.terms)),
        &signature,
        &signer,
    )
    .map_err(|_| QuoteError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::Currency;

    fn key(value: u8) -> SecretKey {
        SecretKey::from_slice(&[value; 32]).unwrap()
    }

    #[test]
    fn signed_quote_binds_terms_and_provider_identities() {
        let secp = Secp256k1::new();
        let source_key = key(2);
        let destination_key = key(3);
        let quote_key = key(4);
        let source = PublicKey::from_secret_key(&secp, &source_key);
        let destination = PublicKey::from_secret_key(&secp, &destination_key);
        let quote = sign(
            &secp,
            &quote_key,
            QuoteTerms {
                quote_id: QuoteId::new(),
                source_amount: FiatAmount::new(Currency::Ngn, 50_000),
                destination_amount: FiatAmount::new(Currency::Kes, 3_000),
                settlement_amount: Millisatoshis(100_000),
                fee_amount: FiatAmount::new(Currency::Ngn, 500),
                expires_at_unix: 2_000,
                source_provider: source.serialize().to_vec(),
                destination_provider: destination.serialize().to_vec(),
            },
        );
        let quote_signer = PublicKey::from_secret_key(&secp, &quote_key);
        assert_eq!(
            verify(&secp, &quote, &quote_signer, &source, &destination, 1_999),
            Ok(())
        );
        assert_eq!(
            verify(&secp, &quote, &quote_signer, &source, &destination, 2_000),
            Err(QuoteError::Expired)
        );
    }

    #[test]
    fn modified_quote_amount_invalidates_signature() {
        let secp = Secp256k1::new();
        let key = key(4);
        let provider = PublicKey::from_secret_key(&secp, &key);
        let mut quote = sign(
            &secp,
            &key,
            QuoteTerms {
                quote_id: QuoteId::new(),
                source_amount: FiatAmount::new(Currency::Ngn, 50_000),
                destination_amount: FiatAmount::new(Currency::Kes, 3_000),
                settlement_amount: Millisatoshis(100_000),
                fee_amount: FiatAmount::new(Currency::Ngn, 500),
                expires_at_unix: 2_000,
                source_provider: provider.serialize().to_vec(),
                destination_provider: provider.serialize().to_vec(),
            },
        );
        quote.terms.settlement_amount = Millisatoshis(100_001);
        assert_eq!(
            verify(&secp, &quote, &provider, &provider, &provider, 1),
            Err(QuoteError::InvalidSignature)
        );
    }
}
