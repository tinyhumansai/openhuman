use super::*;
use crate::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use crate::credentials::provider_credential_source::{
    ProviderCredentialSource, ProviderOAuthTokens,
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tinyinference_providers::oauth::OAuthTokenSet;

static OAUTH_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct RotatingTokenSource {
    refresh_count: AtomicUsize,
    fail_pending_clear: bool,
}

#[async_trait]
impl ProviderCredentialSource for RotatingTokenSource {
    async fn start_openai_oauth(
        &self,
        _config: &crate::config::Config,
    ) -> Result<crate::credentials::ProviderOAuthStart, String> {
        Err("unused in this test".into())
    }

    async fn complete_openai_oauth(
        &self,
        _config: &crate::config::Config,
        _callback: &str,
    ) -> Result<ProviderOAuthTokens, String> {
        Err("unused in this test".into())
    }

    fn import_openai_oauth_from_codex_cli(
        &self,
        _config: &crate::config::Config,
    ) -> Result<ProviderOAuthTokens, String> {
        Err("unused in this test".into())
    }

    fn resolve_openai_oauth(
        &self,
        provider: &str,
        refresh_token: &str,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<Option<OAuthTokenSet>, String> {
        assert_eq!(provider, OPENAI_PROVIDER_KEY);
        assert!(
            expires_at.is_some_and(|expires_at| expires_at <= Utc::now() + Duration::minutes(2))
        );
        self.refresh_count.fetch_add(1, Ordering::SeqCst);
        assert_eq!(refresh_token, "rotating-refresh");
        Ok(Some(OAuthTokenSet {
            access_token: "fresh-access".into(),
            refresh_token: Some("rotated-refresh".into()),
            id_token: None,
            expires_in: 3600,
            issued_at: Utc::now().timestamp().max(0) as u64,
        }))
    }

    async fn clear_openai_oauth_pending(
        &self,
        _config: &crate::config::Config,
    ) -> Result<(), String> {
        if self.fail_pending_clear {
            Err("pending storage unavailable".into())
        } else {
            Ok(())
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_refresh_persists_rotated_token_and_stable_metadata_once() {
    let _env = OAUTH_TEST_LOCK.lock().await;
    let tmp = TempDir::new().unwrap();
    let config = crate::config::Config {
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    let profiles = AuthProfilesStore::new(tmp.path(), false);
    profiles
        .upsert_profile(
            AuthProfile::new_oauth(
                OPENAI_PROVIDER_KEY,
                OPENAI_OAUTH_PROFILE_NAME,
                TokenSet {
                    access_token: "old-access".into(),
                    refresh_token: Some("rotating-refresh".into()),
                    id_token: Some("stable-id".into()),
                    expires_at: Some(Utc::now() + Duration::seconds(30)),
                    token_type: Some("Bearer".into()),
                    scope: Some("openid profile".into()),
                },
            ),
            true,
        )
        .await
        .unwrap();

    let source = std::sync::Arc::new(RotatingTokenSource {
        refresh_count: AtomicUsize::new(0),
        fail_pending_clear: false,
    });
    let previous = crate::credentials::install_provider_credential_source(source.clone());
    let first_config = config.clone();
    let second_config = config.clone();
    let (first, second) = tokio::join!(
        lookup_openai_oauth_credentials(&first_config),
        lookup_openai_oauth_credentials(&second_config),
    );
    assert_eq!(first.unwrap().unwrap().access_token, "fresh-access");
    assert_eq!(second.unwrap().unwrap().access_token, "fresh-access");
    assert_eq!(source.refresh_count.load(Ordering::SeqCst), 1);

    let profile = profiles
        .load()
        .await
        .unwrap()
        .profiles
        .remove(&format!(
            "{OPENAI_PROVIDER_KEY}:{OPENAI_OAUTH_PROFILE_NAME}"
        ))
        .unwrap();
    let token = profile.token_set.unwrap();
    assert_eq!(token.refresh_token.as_deref(), Some("rotated-refresh"));
    assert_eq!(token.id_token.as_deref(), Some("stable-id"));
    assert_eq!(token.scope.as_deref(), Some("openid profile"));

    crate::credentials::clear_provider_credential_source();
    if let Some(previous) = previous {
        crate::credentials::install_provider_credential_source(previous);
    }
}

#[tokio::test]
async fn disconnect_keeps_profile_when_host_cannot_clear_pending_state() {
    let _env = OAUTH_TEST_LOCK.lock().await;
    let tmp = TempDir::new().unwrap();
    let config = crate::config::Config {
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    let profiles = AuthProfilesStore::new(tmp.path(), false);
    profiles
        .upsert_profile(
            AuthProfile::new_oauth(
                OPENAI_PROVIDER_KEY,
                OPENAI_OAUTH_PROFILE_NAME,
                TokenSet {
                    access_token: "access".into(),
                    refresh_token: None,
                    id_token: None,
                    expires_at: None,
                    token_type: Some("Bearer".into()),
                    scope: None,
                },
            ),
            true,
        )
        .await
        .unwrap();
    let source = std::sync::Arc::new(RotatingTokenSource {
        refresh_count: AtomicUsize::new(0),
        fail_pending_clear: true,
    });
    let previous = crate::credentials::install_provider_credential_source(source);

    let error = disconnect_openai_oauth(&config).await.unwrap_err();
    assert!(error.contains("pending storage unavailable"));
    assert!(profiles
        .load()
        .await
        .unwrap()
        .profiles
        .contains_key(&format!(
            "{OPENAI_PROVIDER_KEY}:{OPENAI_OAUTH_PROFILE_NAME}"
        )));

    crate::credentials::clear_provider_credential_source();
    if let Some(previous) = previous {
        crate::credentials::install_provider_credential_source(previous);
    }
}
