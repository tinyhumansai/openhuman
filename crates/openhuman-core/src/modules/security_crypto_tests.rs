use super::*;
use tinysecurity_bus::{CryptoConfig2, KeyringBackend2, KeyringConfig2, Origin};

/// Bytes written by the pre-bus host (`security::encryption::EncryptionKey`)
/// for password `FIXTURE_PASSWORD`. Never regenerate to make a test pass.
const LEGACY_PAYLOAD: &str = include_str!("fixtures/legacy_password_payload.json");
const FIXTURE_PASSWORD: &str = "fixture-password-7328";
const FIXTURE_PLAINTEXT: &[u8] = b"legacy host payload 7328";

fn consent_request(access: KeyringAccess2) -> KeyringConsentRequest2 {
    KeyringConsentRequest2 {
        caller: caller(),
        purpose: "test".into(),
        access,
        name: "secret-name".into(),
        timeout_ms: 1000,
    }
}

fn caller() -> CallerContext {
    CallerContext {
        agent: "agent".into(),
        thread: None,
        flow: None,
        call_id: "call-1".into(),
        origin: Origin::Operator,
        tier: tinysecurity_bus::AccessTier::Full,
    }
}

#[test]
fn consent_is_granted_only_when_the_host_policy_proceeds() {
    for access in [
        KeyringAccess2::Read,
        KeyringAccess2::Write,
        KeyringAccess2::Delete,
    ] {
        let request = consent_request(access);
        assert!(keyring_consent_response(&request, PolicyDecision::Proceed).allowed);
        assert!(!keyring_consent_response(&request, PolicyDecision::Declined).allowed);
        assert!(!keyring_consent_response(&request, PolicyDecision::ConsentRequired).allowed);
    }
}

#[test]
fn legacy_host_payload_shape_is_the_bus_payload_shape() {
    let payload: LegacyPayload = serde_json::from_str(LEGACY_PAYLOAD).unwrap();
    assert_eq!(payload.nonce.len(), 12);
    assert_eq!(payload.salt.len(), 16);
    // The pre-bus host layout is exactly these three byte arrays.
    let value: serde_json::Value = serde_json::from_str(LEGACY_PAYLOAD).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["ciphertext", "nonce", "salt"]);
}

#[test]
fn secret_material_is_redacted_from_debug_output() {
    let request = PasswordRequest2 {
        operation: PasswordOperation2::Encrypt {
            password: "hunter2-secret".into(),
            plaintext: b"plain-secret".to_vec(),
            salt: None,
        },
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("hunter2"));
    assert!(!debug.contains("plain-secret"));
}

fn crypto_config(dir: &std::path::Path) -> CryptoConfig2 {
    CryptoConfig2 {
        max_payload_bytes: 65_536,
        operations_per_minute: 256,
        keyring: Some(KeyringConfig2 {
            service: "openhuman-test-7328".into(),
            backend: KeyringBackend2::EncryptedFile {
                directory: dir.to_string_lossy().into_owned(),
            },
            consent_timeout_ms: 2000,
        }),
        pairing: None,
    }
}

/// The module decrypts the frozen host fixture, and host and module payloads
/// are interchangeable in both directions.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn password_crypt_opens_legacy_host_payloads_and_interoperates() {
    let fx =
        super::super::native_tests::native_fixture_with(Some(crypto_config(fx_dir().path()))).await;
    let payload: LegacyPayload = serde_json::from_str(LEGACY_PAYLOAD).unwrap();
    let opened = password_decrypt(&fx.proxy, FIXTURE_PASSWORD.into(), payload)
        .await
        .unwrap();
    assert_eq!(opened, FIXTURE_PLAINTEXT);

    let sealed = password_encrypt(&fx.proxy, "pw".into(), b"from module".to_vec(), None)
        .await
        .unwrap();
    assert_eq!(
        reference_decrypt("pw", &sealed.salt, &sealed.nonce, &sealed.ciphertext).unwrap(),
        b"from module"
    );

    let wrong = password_decrypt(&fx.proxy, "other".into(), sealed).await;
    assert!(wrong.is_err());
}

/// Independent Argon2id (64 MiB, 3 passes, 1 lane) + AES-256-GCM reference
/// for the legacy layout, so interop does not lean on the module under test.
fn reference_decrypt(
    password: &str,
    salt: &[u8],
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    let params = argon2::Params::new(65536, 3, 1, Some(32)).map_err(|e| e.to_string())?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    aes_gcm::Aes256Gcm::new_from_slice(&key)
        .map_err(|e| e.to_string())?
        .decrypt(aes_gcm::Nonce::from_slice(nonce), ciphertext)
        .map_err(|e| e.to_string())
}

const CORPUS: &str = include_str!("../../../../tests/fixtures/security-crypto-corpus.json");

fn corpus_payload() -> (String, String, LegacyPayload) {
    let v: serde_json::Value = serde_json::from_str(CORPUS).unwrap();
    let a = &v["argon2_aes_gcm"];
    let unhex = |k: &str| -> Vec<u8> {
        let h = a[k].as_str().unwrap();
        (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect()
    };
    (
        a["password"].as_str().unwrap().to_string(),
        a["plaintext"].as_str().unwrap().to_string(),
        LegacyPayload {
            ciphertext: unhex("ciphertext"),
            nonce: unhex("nonce"),
            salt: unhex("salt"),
        },
    )
}

/// Phase-0 characterization rows, now pinned through the module: the committed
/// corpus decrypts, and a wrong password, a tampered ciphertext, a truncated
/// tag, a flipped salt and a flipped nonce are all refused.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn corpus_payload_decrypts_and_tampering_is_refused_through_the_module() {
    let fx =
        super::super::native_tests::native_fixture_with(Some(crypto_config(fx_dir().path()))).await;
    let (password, plaintext, payload) = corpus_payload();
    let opened = password_decrypt(&fx.proxy, password.clone(), payload.clone())
        .await
        .unwrap();
    assert_eq!(opened, plaintext.as_bytes());

    assert!(
        password_decrypt(&fx.proxy, "not-the-password".into(), payload.clone())
            .await
            .is_err()
    );
    let mut flipped = payload.clone();
    flipped.ciphertext[0] ^= 1;
    assert!(password_decrypt(&fx.proxy, password.clone(), flipped)
        .await
        .is_err());
    let mut truncated = payload.clone();
    truncated.ciphertext.pop();
    assert!(password_decrypt(&fx.proxy, password.clone(), truncated)
        .await
        .is_err());
    let mut salt = payload.clone();
    salt.salt[0] ^= 1;
    assert!(password_decrypt(&fx.proxy, password.clone(), salt)
        .await
        .is_err());
    let mut nonce = payload;
    nonce.nonce[0] ^= 1;
    assert!(password_decrypt(&fx.proxy, password, nonce).await.is_err());
}

/// Fresh encryptions never repeat a nonce or salt and carry a 16-byte tag.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn module_encryptions_never_reuse_a_nonce_and_carry_a_gcm_tag() {
    let fx =
        super::super::native_tests::native_fixture_with(Some(crypto_config(fx_dir().path()))).await;
    let a = password_encrypt(&fx.proxy, "pw".into(), b"same plaintext".to_vec(), None)
        .await
        .unwrap();
    let b = password_encrypt(&fx.proxy, "pw".into(), b"same plaintext".to_vec(), None)
        .await
        .unwrap();
    assert_eq!((a.nonce.len(), a.salt.len()), (12, 16));
    assert_ne!(a.nonce, b.nonce);
    assert_ne!(a.ciphertext, b.ciphertext);
    assert_eq!(a.ciphertext.len(), "same plaintext".len() + 16);
}

/// Replaces the former DESIRED-RED row: the module bounds payload size.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn module_refuses_payloads_over_the_configured_limit() {
    let fx =
        super::super::native_tests::native_fixture_with(Some(crypto_config(fx_dir().path()))).await;
    let oversized = vec![0u8; 65_536 + 1];
    assert!(password_encrypt(&fx.proxy, "pw".into(), oversized, None)
        .await
        .is_err());
}

fn fx_dir() -> tempfile::TempDir {
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    tempfile::tempdir_in(target.canonicalize().unwrap()).unwrap()
}

/// Keyring round trip through the module's encrypted-file backend, with the
/// host consent callback served for real (the unit test above pins the
/// declined mapping), the secret is stored, read and deleted.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn keyring_round_trip_through_the_served_consent_callback() {
    let fx =
        super::super::native_tests::native_fixture_with(Some(crypto_config(fx_dir().path()))).await;
    crate::modules::security_host::install(&fx.client)
        .await
        .unwrap();
    let name = "user:token-7328";

    keyring_set(
        &fx.proxy,
        caller(),
        "store a token",
        name,
        b"value-1".to_vec(),
    )
    .await
    .unwrap();
    assert_eq!(
        keyring_get(&fx.proxy, caller(), "read a token", name)
            .await
            .unwrap(),
        Some(b"value-1".to_vec())
    );
    assert!(keyring_delete(&fx.proxy, caller(), "remove a token", name)
        .await
        .unwrap());
    assert_eq!(
        keyring_get(&fx.proxy, caller(), "read a token", name)
            .await
            .unwrap(),
        None
    );
}
