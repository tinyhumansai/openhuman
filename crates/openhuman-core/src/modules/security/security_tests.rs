use super::*;

#[tokio::test]
async fn client_fails_closed_when_module_loading_is_disabled() {
    let mut config = Config::default();
    config.modules.enabled = false;

    assert!(matches!(
        client(&config).await,
        Err(SecurityError::Unavailable(_))
    ));
}

#[test]
fn pairing_unlock_cache_is_process_local_and_generation_bound() {
    let scope = "0123456789abcdef0123456789abcdef";
    UNLOCKED_PAIRING_SCOPES.lock().unwrap().remove(scope);
    assert!(!pairing_scope_is_unlocked(scope, 7));

    remember_unlocked_pairing_scope(scope, 7);
    assert!(pairing_scope_is_unlocked(scope, 7));
    assert!(!pairing_scope_is_unlocked(scope, 8));

    UNLOCKED_PAIRING_SCOPES.lock().unwrap().remove(scope);
    assert!(!pairing_scope_is_unlocked(scope, 7));
}

#[test]
fn logout_clears_pairing_unlock_cache_until_direct_user_unlock() {
    let scope = "fedcba9876543210fedcba9876543210";
    remember_unlocked_pairing_scope(scope, 11);
    assert!(pairing_scope_is_unlocked(scope, 11));

    clear_device_pairing_unlock_cache();

    assert!(!pairing_scope_is_unlocked(scope, 11));
    // Only the direct-user Unlock path can repopulate this process-local gate.
    remember_unlocked_pairing_scope(scope, 11);
    assert!(pairing_scope_is_unlocked(scope, 11));
    UNLOCKED_PAIRING_SCOPES.lock().unwrap().remove(scope);
}

#[test]
fn pairing_unlock_accepts_only_the_module_public_identity_result() {
    let mut response = tinysecurity_bus::DevicePairingResponse2 {
        receipt: None,
        pairing_id: None,
        session_id: None,
        core_public_key: Some("module-core-public-key".into()),
        device_public_key: None,
        frame: Vec::new(),
        expires_at_ms: 0,
    };
    assert!(pairing_unlock_response_is_valid(&response));

    response.pairing_id = Some("unexpected-pairing".into());
    assert!(!pairing_unlock_response_is_valid(&response));
}

#[tokio::test]
#[ignore = "downloads and loads the pinned native TinySecurity release"]
async fn native_cipher_bus_rejects_a_handle_not_minted_by_master_resolution() {
    let mut config = Config::default();
    config.modules.enabled = true;
    let client = client(&config)
        .await
        .expect("pinned TinySecurity module should load with secret callbacks");
    let scope = super::super::ops::security_secret_scope()
        .await
        .expect("the module should retain its persisted audit scope");
    let consumer = super::super::ops::security_cipher_consumer()
        .await
        .expect("the exact module binding should retain its cipher consumer");
    let module_config = super::super::ops::security_module_config(&config)
        .await
        .expect("the module should retain its admitted config");
    let registration_id = module_config["secret_cipher"]["registration_id"]
        .as_str()
        .expect("cipher registration should be present");

    let result = super::secret_cipher::run(
        &client,
        &scope,
        &consumer,
        registration_id,
        "unresolved-master-handle",
        tinysecurity_bus::SecretCipherKind2::Encrypt,
        b"must-not-be-encrypted-with-an-unresolved-master",
    )
    .await;

    assert!(matches!(result, Err(SecurityError::Unavailable(_))));
}

#[tokio::test]
#[ignore = "downloads and loads the pinned native TinySecurity release"]
async fn native_redactor_covers_the_shared_security_fixture() {
    let mut config = Config::default();
    config.modules.enabled = true;
    let client = client(&config)
        .await
        .expect("the pinned TinySecurity module should load");

    for corpus in [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/security-redaction-corpus.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/security-redaction-extended-corpus.json"
        )),
    ] {
        let corpus: serde_json::Value = serde_json::from_str(corpus).unwrap();
        for case in corpus["text"].as_array().unwrap() {
            let kind = case["kind"]
                .as_str()
                .unwrap_or(match case["case"].as_str().unwrap() {
                    "anthropic_key" => "secret",
                    "email" => "pii",
                    _ => "path",
                });
            if !matches!(kind, "secret" | "pii") {
                continue;
            }
            let input = fixture_parts(case, "input_parts", "input");
            let needle = fixture_parts(case, "needle_parts", "needle");
            let output = client
                .redact(tinysecurity_bus::RedactMode::Text, input)
                .await
                .expect("TinySecurity Redact should answer")
                .text;
            assert!(
                !output.contains(&needle),
                "case={} output={output}",
                case["case"]
            );
        }
    }
}

#[tokio::test]
#[ignore = "downloads and loads the pinned native TinySecurity release"]
async fn native_redact_with_covers_encoded_exact_credentials_and_browser_result() {
    let mut config = Config::default();
    config.modules.enabled = true;
    let client = client(&config)
        .await
        .expect("the pinned TinySecurity module should load");
    let secret = "secret+with/slash".to_owned();
    let encoded = [
        secret.clone(),
        "secret%2Bwith%2Fslash".to_owned(),
        "c2VjcmV0K3dpdGgvc2xhc2g=".to_owned(),
        hex::encode(secret.as_bytes()),
    ];
    let rendered = format!("key={secret}; encoded=secret%2Bwith%2Fslash");
    let response = client
        .redact_with(
            tinysecurity_bus::RedactMode::CredentialsOnly,
            rendered,
            encoded,
        )
        .await
        .expect("TinySecurity RedactWith should answer");
    assert!(!response.text.contains(&secret));
    assert!(!response.text.contains("secret%2Bwith%2Fslash"));

    let confirmation = "123e4567-e89b-12d3-a456-426614174000";
    let browser = format!(
        r#"{{"status":"NeedsConfirmation","pending":{{"token":"{confirmation}"}},"page":"api_key={secret}"}}"#
    );
    let response = client
        .redact(tinysecurity_bus::RedactMode::BrowserResult, browser)
        .await
        .expect("TinySecurity BrowserResult should answer");
    assert!(response.text.contains(confirmation));
    assert!(!response.text.contains(&secret));
}

fn fixture_parts(case: &serde_json::Value, parts: &str, fallback: &str) -> String {
    case[parts]
        .as_array()
        .map(|values| values.iter().map(|value| value.as_str().unwrap()).collect())
        .unwrap_or_else(|| case[fallback].as_str().unwrap().to_owned())
}
