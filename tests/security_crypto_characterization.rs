//! Phase-0 crypto characterization (issue #7328, plan Task 1). Committed
//! fixtures written by the current implementations must keep decrypting after
//! the port into the tinysecurity module: Argon2id + AES-256-GCM payloads
//! (now owned by tinysecurity `PasswordCrypt2`; its rows live in
//! `modules/security_crypto_tests.rs`) and the X25519/HKDF/XChaCha20 device tunnel
//! (`security/devices/crypto.rs`). The encrypted-file keyring (`secrets_enc.bin`,
//! `enc2_values.txt`) is decrypted by the test-only master-key seam in
//! `secret_storage/encrypted_file_backend_fixture_tests.rs`; this file pins the
//! digests of those committed bytes so they cannot be regenerated silently.
//! Never regenerate a fixture to make a test pass.

use openhuman_core::security::devices::crypto::{
    base64url_decode, base64url_encode, derive_bootstrap_key, derive_session_keys, DeviceKeypair,
    SessionKeys, TunnelCipher, TunnelRole, HKDF_INFO_C2S, HKDF_INFO_S2C,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

const CORPUS: &str = include_str!("fixtures/security-crypto-corpus.json");
const SECRETS_ENC: &[u8] =
    include_bytes!("../crates/openhuman-core/src/security/secret_storage/fixtures/secrets_enc.bin");
const ENC2_VALUES: &[u8] =
    include_bytes!("../crates/openhuman-core/src/security/secret_storage/fixtures/enc2_values.txt");

fn corpus() -> Value {
    serde_json::from_str(CORPUS).unwrap()
}

fn unhex(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

fn arr32(v: &Value) -> [u8; 32] {
    unhex(v).try_into().unwrap()
}

#[test]
fn hkdf_bootstrap_and_session_key_vectors_are_pinned() {
    let c = corpus();
    let d = &c["devices"];
    let boot = derive_bootstrap_key(&arr32(&d["dh"]), &unhex(&d["salt"]), HKDF_INFO_C2S);
    assert_eq!(boot.to_vec(), unhex(&d["bootstrap_c2s"]));
    let other = derive_bootstrap_key(&arr32(&d["dh"]), &unhex(&d["salt"]), HKDF_INFO_S2C);
    assert_ne!(boot, other, "info tags separate directions");

    let keys = derive_session_keys(
        &arr32(&d["static_dh"]),
        &arr32(&d["eph_dh"]),
        &arr32(&d["client_eph_pub"]),
        &arr32(&d["server_eph_pub"]),
    );
    assert_eq!(keys.c2s.to_vec(), unhex(&d["c2s"]));
    assert_eq!(keys.s2c.to_vec(), unhex(&d["s2c"]));
    assert_eq!(HKDF_INFO_C2S, b"openhuman-tunnel/v1/c2s");
    assert_eq!(HKDF_INFO_S2C, b"openhuman-tunnel/v1/s2c");
}

fn fixture_keys() -> SessionKeys {
    let c = corpus();
    SessionKeys {
        c2s: arr32(&c["devices"]["c2s"]),
        s2c: arr32(&c["devices"]["s2c"]),
    }
}

#[test]
fn committed_tunnel_frames_open_once_and_replays_are_rejected() {
    let c = corpus();
    let frame = unhex(&c["devices"]["server_frame"]);
    assert_eq!(frame[0], 0x02, "current frame version");
    let mut client = TunnelCipher::for_role(TunnelRole::Client, &fixture_keys());
    assert_eq!(
        client.open(&frame).unwrap(),
        c["devices"]["server_frame_plaintext"]
            .as_str()
            .unwrap()
            .as_bytes()
    );
    assert!(client.open(&frame).unwrap_err().contains("replayed nonce"));

    // The sealing side cannot open its own direction.
    let mut server = TunnelCipher::for_role(TunnelRole::Server, &fixture_keys());
    assert!(server
        .open(&frame)
        .unwrap_err()
        .contains("authentication failed"));
}

#[test]
fn malformed_tunnel_frames_are_rejected_with_distinct_errors() {
    let c = corpus();
    let good = unhex(&c["devices"]["server_frame"]);
    let mut client = TunnelCipher::for_role(TunnelRole::Client, &fixture_keys());
    assert!(client.open(&[]).unwrap_err().contains("empty frame"));
    let mut legacy = good.clone();
    legacy[0] = 0x01;
    assert!(client
        .open(&legacy)
        .unwrap_err()
        .contains("UnsupportedFrameVersion"));
    let mut unknown = good.clone();
    unknown[0] = 0x7f;
    assert!(client
        .open(&unknown)
        .unwrap_err()
        .contains("unsupported frame version"));
    assert!(client.open(&good[..10]).unwrap_err().contains("too short"));
    let mut tampered = good.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(client
        .open(&tampered)
        .unwrap_err()
        .contains("authentication failed"));
    // None of the failures consumed the nonce: the intact frame still opens.
    assert!(client.open(&good).is_ok());
}

#[test]
fn x25519_agreement_matches_and_rejects_bad_peers() {
    let a = DeviceKeypair::generate();
    let b = DeviceKeypair::generate();
    assert_eq!(
        a.derive_shared_secret(&b.pubkey_b64).unwrap(),
        b.derive_shared_secret(&a.pubkey_b64).unwrap()
    );
    assert!(a.derive_shared_secret("!!not-base64!!").is_err());
    assert!(a
        .derive_shared_secret(&base64url_encode(&[7u8; 31]))
        .unwrap_err()
        .contains("32 bytes"));
    // A low-order point yields an all-zero secret, which must be refused.
    assert!(a
        .derive_shared_secret(&base64url_encode(&[0u8; 32]))
        .is_err());
    let raw = base64url_decode(&a.pubkey_b64).unwrap();
    assert_eq!(raw.len(), 32);
    assert!(!a.pubkey_b64.contains('='), "unpadded base64url");
}

#[test]
fn encrypted_file_keyring_fixture_bytes_are_unchanged() {
    // Decryption of these bytes is asserted beside the backend (the master-key
    // seam is test-only). Pinning the digests stops a silent regeneration.
    let c = corpus();
    let digest = |bytes: &[u8]| hex::encode(Sha256::digest(bytes));
    assert_eq!(
        digest(SECRETS_ENC),
        c["keyring"]["secrets_enc_sha256"].as_str().unwrap()
    );
    assert_eq!(
        digest(ENC2_VALUES),
        c["keyring"]["enc2_values_sha256"].as_str().unwrap()
    );
    for line in std::str::from_utf8(ENC2_VALUES).unwrap().lines() {
        assert!(line.starts_with("enc2:"));
    }
}
