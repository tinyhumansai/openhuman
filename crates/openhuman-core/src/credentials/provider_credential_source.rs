//! Host-owned provider credential acquisition and refresh operations.
//!
//! Provider OAuth needs the host's platform secret store and filesystem
//! conventions. Core owns the resulting auth profile, while a host installed
//! through this port performs the provider-specific exchange and refresh.

use std::sync::{Arc, LazyLock, RwLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tinyinference_providers::oauth::OAuthTokenSet;

use crate::config::Config;

/// Values returned by the host when starting OpenAI Codex OAuth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOAuthStart {
    /// Provider authorization URL.
    pub auth_url: String,
    /// One-time state bound to the pending PKCE verifier.
    pub state: String,
    /// Redirect URI the provider must call.
    pub redirect_uri: String,
}

/// Tokens returned by a provider OAuth operation and optional account
/// metadata the provider supplies alongside those tokens.
#[derive(Clone)]
pub struct ProviderOAuthTokens {
    /// OAuth token set to persist in the provider profile.
    pub token: OAuthTokenSet,
    /// Account identifier when available outside the token claims.
    pub account_id: Option<String>,
}

/// Host-owned source for provider OAuth credentials.
#[async_trait]
pub trait ProviderCredentialSource: Send + Sync + 'static {
    /// Start OpenAI Codex OAuth and persist its short-lived pending state.
    async fn start_openai_oauth(&self, config: &Config) -> Result<ProviderOAuthStart, String>;

    /// Exchange the callback for OAuth tokens, consuming pending state.
    async fn complete_openai_oauth(
        &self,
        config: &Config,
        callback: &str,
    ) -> Result<ProviderOAuthTokens, String>;

    /// Import a token set from the local Codex CLI credential file.
    fn import_openai_oauth_from_codex_cli(
        &self,
        config: &Config,
    ) -> Result<ProviderOAuthTokens, String>;

    /// Resolve an OpenAI OAuth credential that may need refreshing. The host
    /// owns the provider refresh policy and request; the core supplies the
    /// stored refresh token and expiry, then persists a returned token set.
    /// `Ok(None)` means the current token remains usable.
    fn resolve_openai_oauth(
        &self,
        provider: &str,
        refresh_token: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<OAuthTokenSet>, String>;

    /// Clear pending OAuth state after the core disconnects the profile.
    async fn clear_openai_oauth_pending(&self, config: &Config) -> Result<(), String>;
}

static SOURCE: LazyLock<RwLock<Option<Arc<dyn ProviderCredentialSource>>>> =
    LazyLock::new(|| RwLock::new(None));

/// Install a host's provider OAuth implementation. Re-installing replaces the
/// previous implementation, matching other process-wide host ports.
pub fn install_provider_credential_source(
    source: Arc<dyn ProviderCredentialSource>,
) -> Option<Arc<dyn ProviderCredentialSource>> {
    (*SOURCE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()))
    .replace(source)
}

/// Clear the host port. Intended for tests that must restore process-global
/// registration after exercising credential resolution.
#[doc(hidden)]
pub fn clear_provider_credential_source() -> Option<Arc<dyn ProviderCredentialSource>> {
    std::mem::take(
        &mut *SOURCE
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// Return the currently installed provider OAuth implementation, if any.
pub fn provider_credential_source() -> Option<Arc<dyn ProviderCredentialSource>> {
    SOURCE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}
