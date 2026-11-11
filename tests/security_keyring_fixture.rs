//! Committed encrypted-file keyring fixtures stay byte-compatible.

use serde_json::Value;
use sha2::{Digest, Sha256};

const CORPUS: &str = include_str!("fixtures/security-crypto-corpus.json");
const SECRETS_ENC: &[u8] =
    include_bytes!("../crates/openhuman-core/src/security/secret_storage/fixtures/secrets_enc.bin");
const ENC2_VALUES: &[u8] =
    include_bytes!("../crates/openhuman-core/src/security/secret_storage/fixtures/enc2_values.txt");

fn corpus() -> Value {
    serde_json::from_str(CORPUS).expect("valid keyring fixture characterization")
}

#[test]
fn committed_encrypted_file_keyring_fixture_bytes_are_unchanged() {
    let keyring = &corpus()["keyring"];
    let digest = |bytes: &[u8]| hex::encode(Sha256::digest(bytes));
    assert_eq!(
        digest(SECRETS_ENC),
        keyring["secrets_enc_sha256"]
            .as_str()
            .expect("fixture digest")
    );
    assert_eq!(
        digest(ENC2_VALUES),
        keyring["enc2_values_sha256"]
            .as_str()
            .expect("fixture digest")
    );
    for line in std::str::from_utf8(ENC2_VALUES).unwrap().lines() {
        assert!(line.starts_with("enc2:"));
    }
}
