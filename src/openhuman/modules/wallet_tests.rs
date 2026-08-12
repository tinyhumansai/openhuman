//! Tests for the wallet module client.
//!
//! Nothing here loads a module. What is testable without one is the part that
//! decides what a caller does next: how a bus failure is classified, that the
//! unavailable path is reached without a broker, and — most importantly — that
//! the local signing produces what the module asked for. The round trips
//! themselves are covered where they can be honest: `tinywallet`'s own loader
//! E2E, which drives a real module over a real broker.

use tinywallet::wire::{Scheme, Signature, SigningPayload, TransactionSpec};
use tinywallet::Chain;

use super::{classify, sign_payload, WalletCallError};
use crate::openhuman::config::Config;

/// The BIP-39 test vector mnemonic. Never use it for real funds.
const VECTOR: &str = "abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon about";

/// A config with modules enabled but nothing fetchable.
fn offline_config() -> Config {
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config
}

/// A bus failure carrying `name`.
fn failure(name: &str) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: name.to_string(),
        message: "something went wrong".to_string(),
    }
}

fn evm_secret() -> Vec<u8> {
    tinywallet::key::derive(Chain::Evm, VECTOR, "m/44'/60'/0'/0/0")
        .expect("the vector mnemonic derives")
        .secret_bytes()
        .to_vec()
}

#[test]
fn an_invalid_input_is_reported_as_something_the_caller_can_fix() {
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinywallet.Error.InvalidInput")),
        WalletCallError::InvalidInput(_)
    ));
}

#[test]
fn a_build_failure_is_not_an_input_error() {
    // Telling a caller its transaction was wrong when the encoder broke points
    // at the wrong fix.
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinywallet.Error.BuildFailed")),
        WalletCallError::Failed(_)
    ));
}

#[test]
fn an_unsupported_chain_reads_as_unavailable_not_invalid() {
    // Defensive: the current module never emits this name — an unrecognised
    // transaction shape comes back as `InvalidInput` instead. The arm exists
    // so a future module that does distinguish "chain not compiled in" from
    // "bad request" is classified as a missing capability rather than as the
    // caller's fault.
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinywallet.Error.UnsupportedChain")),
        WalletCallError::Unavailable(_)
    ));
}

#[test]
fn an_unrecognised_wire_name_is_a_failure_not_an_input_error() {
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinywallet.Error.SomethingNewer")),
        WalletCallError::Failed(_)
    ));
}

#[test]
fn a_missing_module_reads_as_unavailable() {
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinybus.Error.ModuleUnavailable")),
        WalletCallError::Unavailable(_)
    ));
}

#[test]
fn every_error_renders_as_its_message() {
    assert_eq!(
        WalletCallError::InvalidInput("bad recipient".to_string()).to_string(),
        "bad recipient"
    );
    for error in [
        WalletCallError::Unavailable("gone".to_string()),
        WalletCallError::Failed("encoder stopped".to_string()),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn a_prehash_payload_is_signed_without_being_hashed_again() {
    // The single most dangerous confusion in this file: hashing a digest a
    // second time yields a valid signature over the wrong preimage, which the
    // chain accepts as a different transaction or rejects with no explanation.
    // Verified by recovering the signature against the digest itself.
    use k256::ecdsa::signature::hazmat::PrehashVerifier as _;
    use k256::ecdsa::{Signature as K256Signature, SigningKey, VerifyingKey};

    let secret = evm_secret();
    let digest = [0x42u8; 32];
    let payload = SigningPayload {
        bytes_hex: "42".repeat(32),
        scheme: Scheme::Secp256k1Prehash,
    };

    let Signature::Secp256k1 { rs_hex, .. } = sign_payload(&payload, &secret).unwrap() else {
        panic!("a prehash payload must produce a secp256k1 signature");
    };

    let raw: Vec<u8> = (0..rs_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&rs_hex[i..i + 2], 16).unwrap())
        .collect();
    let signature = K256Signature::from_slice(&raw).unwrap();
    let verifying: VerifyingKey = *SigningKey::from_slice(&secret).unwrap().verifying_key();

    verifying
        .verify_prehash(&digest, &signature)
        .expect("the signature must verify against the digest, not a rehash of it");
}

#[test]
fn a_prehash_payload_that_is_not_thirty_two_bytes_is_refused() {
    let payload = SigningPayload {
        bytes_hex: "42".repeat(16),
        scheme: Scheme::Secp256k1Prehash,
    };
    assert!(matches!(
        sign_payload(&payload, &evm_secret()),
        Err(WalletCallError::Failed(_))
    ));
}

#[test]
fn an_ed25519_payload_is_signed_over_the_whole_message() {
    // ed25519 hashes internally, so the payload is the message. Verified
    // against the public key rather than merely checked for a length.
    use ed25519_dalek::{Signature as EdSignature, SigningKey, Verifier as _};

    let derived = tinywallet::key::derive(Chain::Solana, VECTOR, "m/44'/501'/0'/0'").unwrap();
    let secret = derived.secret_bytes();
    let message = b"a solana message that is clearly longer than thirty-two bytes";

    let payload = SigningPayload {
        bytes_hex: message.iter().fold(String::new(), |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        }),
        scheme: Scheme::Ed25519,
    };

    let Signature::Ed25519 { signature_hex } = sign_payload(&payload, secret).unwrap() else {
        panic!("an ed25519 payload must produce an ed25519 signature");
    };

    let raw: Vec<u8> = (0..signature_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&signature_hex[i..i + 2], 16).unwrap())
        .collect();
    let key: [u8; 32] = secret.try_into().unwrap();
    SigningKey::from_bytes(&key)
        .verifying_key()
        .verify(message, &EdSignature::from_slice(&raw).unwrap())
        .expect("the signature must verify over the message");
}

#[test]
fn a_malformed_payload_from_the_module_is_refused_rather_than_signed() {
    for bytes_hex in ["abc", "zz".repeat(32).as_str(), "aéb"] {
        let payload = SigningPayload {
            bytes_hex: bytes_hex.to_string(),
            scheme: Scheme::Secp256k1Prehash,
        };
        assert!(
            matches!(
                sign_payload(&payload, &evm_secret()),
                Err(WalletCallError::Failed(_))
            ),
            "{bytes_hex:?} should be refused"
        );
    }
}

#[tokio::test]
async fn a_disabled_host_reports_unavailable_without_starting_a_broker() {
    let mut config = offline_config();
    config.modules.enabled = false;

    let spec = TransactionSpec::Evm {
        to: "0x3535353535353535353535353535353535353535".to_string(),
        value_wei: "1".to_string(),
        data_hex: "0x".to_string(),
        nonce: 0,
        gas_limit: 21_000,
        gas_price_wei: "1".to_string(),
        chain_id: 1,
    };

    assert!(matches!(
        super::sign_transaction(&config, &spec, &evm_secret(), &[0x02; 33]).await,
        Err(WalletCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::ensure_ready(&config).await,
        Err(WalletCallError::Unavailable(_))
    ));
}
