//! Storing, removing, and listing provider (non-app-session) credentials.

use serde_json::json;

use crate::config::Config;
use crate::core::Outcome;
use crate::credentials::session_state::{
    parse_fields_value, profile_name_or_default, summarize_auth_profile,
};
use crate::credentials::{normalize_provider, AuthService, APP_SESSION_PROVIDER};

pub async fn store_provider_credentials(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
    token: Option<String>,
    fields: Option<serde_json::Value>,
    set_active: Option<bool>,
) -> Result<Outcome<super::super::responses::AuthProfileSummary>, String> {
    let provider = provider.trim().to_string();
    if provider.is_empty() {
        return Err("provider is required".to_string());
    }

    let profile_name = profile_name_or_default(profile);
    let mut metadata = parse_fields_value(fields)?;
    let token = token
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| metadata.get("token").cloned())
        .or_else(|| metadata.get("api_key").cloned())
        .unwrap_or_default();
    if token.is_empty() && metadata.is_empty() {
        return Err("provide at least one credential via token or fields".to_string());
    }
    metadata.remove("token");

    let auth = AuthService::from_config(config);
    let stored = auth
        .store_provider_token(
            &provider,
            profile_name,
            &token,
            metadata,
            set_active.unwrap_or(true),
        )
        .await
        .map_err(|e| e.to_string())?;
    // A freshly-stored key supersedes any prior auth rejection for this
    // provider — clear the recorded BYO auth error so the AI-settings notice
    // disappears and the notification latch re-arms (a future rejection will
    // notify again). Credentials are keyed `provider:<slug>`; the auth-error
    // registry is keyed by the bare provider slug used by the chat factory.
    clear_provider_auth_error(&provider);
    Ok(Outcome::single_log(
        summarize_auth_profile(&stored),
        "provider credentials stored",
    ))
}

/// Clear any recorded BYO provider auth error for a credentials `provider`
/// key. Strips the `provider:` namespace prefix so the lookup matches the
/// bare slug (`openrouter`) the inference classifier records under.
fn clear_provider_auth_error(provider: &str) {
    let slug = provider.strip_prefix("provider:").unwrap_or(provider);
    crate::inference::auth_error_registry::clear(slug);
}

pub async fn remove_provider_credentials(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
) -> Result<Outcome<serde_json::Value>, String> {
    let profile_name = profile_name_or_default(profile);
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(provider, profile_name)
        .await
        .map_err(|e| e.to_string())?;
    // Removing the key clears any recorded BYO auth error for this provider —
    // there is no longer a key to be "rejected", so the stale notice must go.
    clear_provider_auth_error(provider);
    Ok(Outcome::single_log(
        json!({
            "removed": removed,
            "provider": provider,
            "profile": profile_name,
        }),
        "provider credentials removed",
    ))
}

pub async fn list_provider_credentials(
    config: &Config,
    provider_filter: Option<String>,
) -> Result<Outcome<Vec<super::super::responses::AuthProfileSummary>>, String> {
    let auth = AuthService::from_config(config);
    let provider_filter = provider_filter
        .map(|provider| normalize_provider(&provider))
        .transpose()
        .map_err(|e| e.to_string())?;
    let profiles = auth.load_profiles().await.map_err(|e| e.to_string())?;
    let mut items = profiles
        .profiles
        .values()
        .filter(|profile| profile.provider != APP_SESSION_PROVIDER)
        .filter(|profile| {
            provider_filter
                .as_ref()
                .is_none_or(|provider| profile.provider == *provider)
        })
        .map(summarize_auth_profile)
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.profile_name.cmp(&b.profile_name))
    });

    Ok(Outcome::single_log(items, "provider credentials listed"))
}

/// List credentials whose provider key starts with `prefix`.
///
/// Pure prefix variant of [`list_provider_credentials`] for namespaces
/// that group multiple providers under a common stem (e.g.
/// `"channel:"` covers `channel:telegram:managed_dm`,
/// `channel:slack:bot_token`, …). The exact-match filter on
/// `list_provider_credentials` cannot express this without enumerating
/// every concrete provider key up front.
pub async fn list_provider_credentials_by_prefix(
    config: &Config,
    prefix: &str,
) -> Result<Vec<super::super::responses::AuthProfileSummary>, String> {
    let auth = AuthService::from_config(config);
    let profiles = auth.load_profiles().await.map_err(|e| e.to_string())?;
    let mut items = profiles
        .profiles
        .values()
        .filter(|profile| profile.provider != APP_SESSION_PROVIDER)
        .filter(|profile| profile.provider.starts_with(prefix))
        .map(summarize_auth_profile)
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.profile_name.cmp(&b.profile_name))
    });
    Ok(items)
}
