use super::*;

#[tokio::test]
async fn config_secret_encryption_fails_closed_without_tinysecurity() {
    #[cfg(feature = "security-module")]
    let _absent = crate::modules::security::test_host::absent();
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.modules.enabled = false;
    config.secrets.encrypt = true;
    config.api_key = Some("must-not-be-written-in-plaintext".into());

    let error = encrypt_config_secrets(&mut config)
        .await
        .expect_err("config save must require TinySecurity cipher operations");

    // `to_string` is the outermost context only; the cause is in the chain.
    assert!(
        format!("{error:#}").contains("security module unavailable"),
        "{error:#}"
    );
    assert!(!dir.path().join(".secret_key").exists());
    assert_eq!(
        config.api_key.as_deref(),
        Some("must-not-be-written-in-plaintext")
    );
}

#[tokio::test]
async fn failed_storage_url_decryption_keeps_the_sealed_value() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.modules.enabled = false;
    config.secrets.encrypt = true;
    let sealed = format!("enc2:{}", "00".repeat(32));
    config.storage.url = Some(sealed.clone());

    assert!(!decrypt_config_secrets(&mut config, dir.path())
        .await
        .unwrap());
    assert_eq!(config.storage.url.as_deref(), Some(sealed.as_str()));
}

#[tokio::test]
async fn disabled_config_secret_encryption_is_a_plaintext_noop() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.secrets.encrypt = false;
    config.api_key = Some("plaintext-key".into());

    encrypt_config_secrets(&mut config).await.unwrap();
    assert_eq!(config.api_key.as_deref(), Some("plaintext-key"));
}

/// The Settings save RPC reports `config_save_error`, which prints only the
/// outermost context. It must still carry why sealing failed, not a bare
/// "Failed to encrypt api_key".
#[tokio::test]
async fn a_failed_secret_encryption_reports_the_field_and_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    // A store without its configuration cannot reach TinySecurity.
    let store = crate::security::secret_storage::SecretStore::new(dir.path(), true);
    let mut value = Some("sk-must-not-leak".to_string());

    let error = encrypt_optional_secret(&store, &mut value, "api_key")
        .await
        .expect_err("sealing without TinySecurity must fail");
    let shown = crate::config::ops::config_save_error(error);

    assert!(shown.starts_with("Failed to encrypt api_key: "), "{shown}");
    assert!(
        shown.contains("TinySecurity configuration is unavailable"),
        "{shown}"
    );
    assert!(!shown.contains("sk-must-not-leak"), "{shown}");
}

// ── secrets.unattended_unlock on a headless host ───────────────────────────

/// The two headless-host tests share the process gate and the admitted module:
/// one approves whatever grant it raised, so they must not overlap.
#[cfg(feature = "security-module")]
static HEADLESS_HOST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// These tests load the real TinySecurity module. They run only where the
/// runner staged it (`OPENHUMAN_TEST_MODULES_STAGED=1`, as CI does), never
/// downloading it from a clean checkout.
#[cfg(feature = "security-module")]
fn native_module_staged() -> bool {
    let staged = std::env::var("OPENHUMAN_TEST_MODULES_STAGED").as_deref() == Ok("1");
    if !staged {
        eprintln!("skipped: the native TinySecurity test module is not staged");
    }
    staged
}

#[cfg(feature = "security-module")]
const UNATTENDED_FIXTURE_KEY: &str = "local-ai-unattended-fixture-value";

#[cfg(feature = "security-module")]
fn native_cipher_fixture(dir: &Path, unattended_unlock: bool) -> Config {
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = Config {
        config_path: dir.join("config.toml"),
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        ..Config::default()
    };
    config.secrets.encrypt = true;
    config.secrets.unattended_unlock = unattended_unlock;
    config.local_ai.api_key = Some(UNATTENDED_FIXTURE_KEY.into());
    config
}

/// What a booted serve host has: the security module admitted for this config
/// and the process approval gate, with no chat surface and nobody to answer.
#[cfg(feature = "security-module")]
async fn headless_gate(config: &Config) -> std::sync::Arc<crate::security::approval::ApprovalGate> {
    crate::modules::security::ensure_loaded(config)
        .await
        .expect("admit the native cipher module");
    crate::core::bus::init().await.expect("approval events");
    crate::security::approval::ApprovalGate::init_global(
        config.clone(),
        format!("session-{}", uuid::Uuid::new_v4()),
    )
}

/// A headless `serve` host with `secrets.unattended_unlock` and an
/// operator-supplied keyring key seals and reopens `local_ai.api_key` with
/// nobody answering the "Unlock configuration secrets" grant. Before the
/// setting existed the grant parked unrouted until it expired, and every
/// config load timed out.
#[cfg(feature = "security-module")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unattended_unlock_round_trips_an_encrypted_local_ai_key_without_a_person() {
    if !native_module_staged() {
        return;
    }
    let _serial = HEADLESS_HOST.lock().await;
    let _explicit = crate::modules::security::test_host::explicit_consent();
    let _operator_key = crate::modules::security::unattended_unlock::assume_operator_key();
    let dir = tempfile::tempdir().unwrap();
    let config = native_cipher_fixture(dir.path(), true);
    let _gate = headless_gate(&config).await;

    let opened = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut sealed = config.clone();
        encrypt_config_secrets(&mut sealed)
            .await
            .expect("seal local_ai.api_key without a person");
        let raw = sealed.local_ai.api_key.clone().expect("sealed value");
        assert!(
            crate::security::secret_storage::SecretStore::is_encrypted(&raw),
            "local_ai.api_key must be sealed on save"
        );
        assert!(!raw.contains(UNATTENDED_FIXTURE_KEY));
        let mut opened = sealed.clone();
        decrypt_config_secrets(&mut opened, dir.path())
            .await
            .expect("open local_ai.api_key without a person");
        opened
    })
    .await
    .expect("the unlock must not wait for a person");

    assert_eq!(
        opened.local_ai.api_key.as_deref(),
        Some(UNATTENDED_FIXTURE_KEY)
    );
}

/// With the setting off (the default) the same host still parks the grant for
/// a person, operator key or not, and the save continues only once someone
/// answers it.
#[cfg(feature = "security-module")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_unattended_unlock_the_secret_grant_still_parks_for_a_person() {
    use crate::security::approval::ApprovalDecision;

    if !native_module_staged() {
        return;
    }
    let _serial = HEADLESS_HOST.lock().await;
    let _explicit = crate::modules::security::test_host::explicit_consent();
    let _operator_key = crate::modules::security::unattended_unlock::assume_operator_key();
    let dir = tempfile::tempdir().unwrap();
    let config = native_cipher_fixture(dir.path(), false);
    let gate = headless_gate(&config).await;

    let mut sealed = config.clone();
    let save = tokio::spawn(async move { encrypt_config_secrets(&mut sealed).await });
    let pending = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(pending) = gate
                .list_pending()
                .unwrap_or_default()
                .into_iter()
                .find(|pending| pending.tool_name == "native_secret_grant")
            {
                break pending;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the grant parks for a person");
    assert!(!save.is_finished(), "the save must wait for the person");

    // Approve rather than deny: the process gate is shared, and a pending
    // grant found here may belong to another native-cipher test, which an
    // approval leaves working where a refusal would break it.
    gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
        .expect("approve the parked grant");
    tokio::time::timeout(std::time::Duration::from_secs(30), save)
        .await
        .expect("an answer ends the save")
        .expect("save task")
        .expect("the save succeeds once a person approves");
}
