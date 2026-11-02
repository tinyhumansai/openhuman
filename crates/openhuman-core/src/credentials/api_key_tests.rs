use super::*;
use crate::credentials::backend_credential::{resolve_backend_credential, BackendCredential};

fn config_in(dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

#[tokio::test]
async fn store_then_get_round_trips_and_marks_kind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_in(dir.path());
    let profile = store_api_key(&config, "  th_live_abc  ")
        .await
        .expect("store");
    assert_eq!(profile.provider, API_KEY_PROVIDER);
    assert_eq!(
        profile.metadata.get(API_KEY_KIND_META).map(String::as_str),
        Some(API_KEY_KIND)
    );
    assert_eq!(
        get_api_key(&config).await.expect("get").as_deref(),
        Some("th_live_abc")
    );
    assert!(has_api_key(&config).await);
}

#[tokio::test]
async fn blank_key_is_rejected_and_nothing_is_stored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_in(dir.path());
    assert!(store_api_key(&config, "   ").await.is_err());
    assert!(!has_api_key(&config).await);
    assert_eq!(get_api_key(&config).await.expect("get"), None);
}

#[tokio::test]
async fn clear_removes_the_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_in(dir.path());
    store_api_key(&config, "th_x").await.expect("store");
    assert!(clear_api_key(&config).await.expect("clear"));
    assert!(!has_api_key(&config).await);
    assert!(!clear_api_key(&config).await.expect("clear again"));
}

/// Regression: an ordinary provider profile named `"api-key"` — the generic
/// `auth_store_provider_credentials` RPC/CLI allow that provider name like
/// any other — must not be accepted as the TinyHumans runtime key just
/// because it lives under the same provider id. Only a profile carrying the
/// [`API_KEY_KIND_META`]`=`[`API_KEY_KIND`] marker (written by
/// [`store_api_key`]) counts.
#[tokio::test]
async fn a_provider_profile_named_api_key_without_the_marker_is_not_accepted() {
    use crate::credentials::AuthService;

    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_in(dir.path());
    // Simulates the generic provider-credentials path: no `kind` metadata.
    AuthService::from_config(&config)
        .store_provider_token(
            API_KEY_PROVIDER,
            crate::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "not-actually-a-tinyhumans-key",
            std::collections::HashMap::new(),
            true,
        )
        .await
        .expect("store unmarked provider profile");

    assert_eq!(
        get_api_key(&config).await.expect("get"),
        None,
        "an unmarked provider profile named api-key must not be read back as the runtime key"
    );
    assert!(!has_api_key(&config).await);
}

#[tokio::test]
async fn api_key_wins_over_an_expired_session() {
    use crate::credentials::session_state::SESSION_EXPIRES_AT_META;
    use crate::credentials::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_in(dir.path());
    let mut meta = std::collections::HashMap::new();
    meta.insert(
        SESSION_EXPIRES_AT_META.to_string(),
        (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
    );
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "jwt",
            meta,
            true,
        )
        .await
        .expect("store session");
    assert!(matches!(
        resolve_backend_credential(&config).await,
        Err(e) if e.contains("SESSION_EXPIRED")
    ));

    store_api_key(&config, "th_key").await.expect("store key");
    assert_eq!(
        resolve_backend_credential(&config)
            .await
            .expect("credential"),
        BackendCredential::ApiKey("th_key".to_string())
    );
}
