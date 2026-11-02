//! Auth-profile credential lookup for `<slug>` cloud providers, including the
//! OpenAI OAuth fallback and the scoped legacy `config.api_key` fallback.

use super::*;
use crate::config::ops::legacy_inference_slug;
use crate::credentials::AuthService;
use std::collections::HashMap;

type ReadOnlyProfileCredential = (
    String,
    bool,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<String>,
);

/// Credential behavior used by shared model construction. Runtime callers keep
/// the established mutable auth-profile store path; doctor inspection uses the
/// selected profile's read-only snapshot and never refreshes credentials.
#[derive(Clone, Copy)]
pub(super) enum CredentialAccess<'a> {
    Runtime(&'a RuntimeCredentialSnapshot),
    Inspection,
}

/// A per-session snapshot prepared before the synchronous TinyAgents
/// `ModelFactory::build` callback. It is owned by one short-lived model
/// factory, never cached process-wide.
pub(crate) struct RuntimeCredentialSnapshot {
    keys: HashMap<String, Result<String, String>>,
    openai_oauth: Result<Option<crate::credentials::openai_oauth::OpenAiOAuthCredentials>, String>,
    openai_bearer_is_oauth: bool,
    backend_session_active: bool,
}

impl Default for RuntimeCredentialSnapshot {
    fn default() -> Self {
        Self {
            keys: HashMap::new(),
            openai_oauth: Ok(None),
            openai_bearer_is_oauth: false,
            backend_session_active: false,
        }
    }
}

impl RuntimeCredentialSnapshot {
    pub(crate) async fn resolve(config: &Config) -> Self {
        let mut keys = HashMap::new();
        for slug in config
            .cloud_providers
            .iter()
            .map(|entry| entry.slug.as_str())
        {
            if keys.contains_key(slug) {
                continue;
            }
            let result = lookup_key_for_slug(slug, config)
                .await
                .map_err(|error| error.to_string());
            keys.insert(slug.to_owned(), result);
        }
        if let Some(route) = config.ephemeral_route.as_ref() {
            keys.insert(
                crate::config::schema::EPHEMERAL_ROUTE_SLUG.to_owned(),
                Ok(route.api_key.trim().to_owned()),
            );
        }
        let openai_oauth =
            crate::credentials::openai_oauth::lookup_openai_oauth_credentials(config).await;
        let openai_bearer_is_oauth = openai_bearer_is_oauth(config).await;
        let backend_session_active = backend_session_active_snapshot(config).await;
        Self {
            keys,
            openai_oauth,
            openai_bearer_is_oauth,
            backend_session_active,
        }
    }

    pub(super) fn key(&self, slug: &str) -> anyhow::Result<String> {
        self.keys
            .get(slug)
            .cloned()
            .unwrap_or_else(|| Ok(String::new()))
            .map_err(anyhow::Error::msg)
    }

    pub(super) fn openai_bearer_is_oauth(&self) -> bool {
        self.openai_bearer_is_oauth
    }

    pub(super) fn backend_session_active(&self) -> bool {
        self.backend_session_active
    }

    pub(super) fn codex_routing(
        &self,
        slug: &str,
        endpoint: &str,
        bearer_key: &str,
    ) -> Result<tinyinference_llm::providers::openai::codex::OpenAiCodexRouting, String> {
        let credentials = if slug == "openai" {
            match &self.openai_oauth {
                Ok(credentials) => credentials.clone(),
                Err(error) if !bearer_key.trim().is_empty() => {
                    log::warn!("[providers][openai-codex] oauth metadata unavailable; continuing with standard bearer key: {error}");
                    None
                }
                Err(error) => {
                    return Err(format!(
                        "[chat-factory] openai oauth lookup failed: {error}"
                    ))
                }
            }
        } else {
            None
        };
        let use_oauth = self.openai_bearer_is_oauth && credentials.is_some();
        let account_id = credentials
            .filter(|_| use_oauth)
            .and_then(|credentials| credentials.account_id);
        let mut route = tinyinference_llm::providers::openai::codex::resolve_codex_routing(
            slug,
            endpoint,
            use_oauth,
            account_id.as_deref(),
        );
        if !route.using_oauth {
            route.endpoint = endpoint.to_string();
        }
        Ok(route)
    }
}

async fn backend_session_active_snapshot(config: &Config) -> bool {
    let state_dir = config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|dirs| dirs.home_dir().join(".openhuman"))
                .unwrap_or_else(|| std::path::PathBuf::from(".openhuman"))
        });
    if crate::credentials::api_key::has_api_key_in(&state_dir, config.secrets.encrypt).await {
        return true;
    }
    if crate::platform::llm_gate::is_signed_out() {
        return false;
    }
    AuthService::new(&state_dir, config.secrets.encrypt)
        .get_provider_bearer_token(crate::credentials::APP_SESSION_PROVIDER, None)
        .await
        .ok()
        .flatten()
        .is_some_and(|token| !token.trim().is_empty())
}

impl CredentialAccess<'_> {
    pub(super) fn is_inspection(self) -> bool {
        matches!(self, Self::Inspection)
    }

    pub(super) fn lookup_key_for_slug(self, slug: &str) -> anyhow::Result<String> {
        match self {
            Self::Runtime(snapshot) => snapshot.key(slug),
            Self::Inspection => anyhow::bail!("inspection credential was not pre-resolved"),
        }
    }

    pub(super) fn openai_bearer_is_oauth(self) -> bool {
        match self {
            Self::Runtime(snapshot) => snapshot.openai_bearer_is_oauth(),
            Self::Inspection => false,
        }
    }
}

/// Resolve a configured cloud slug after asynchronously taking this model
/// factory's credential snapshot.
pub(crate) async fn resolve_cloud_slug_for<'a>(
    role: &str,
    slug: &str,
    model: &str,
    config: &'a Config,
    extensions: Option<&crate::core::slots::Extensions>,
) -> anyhow::Result<super::cloud_slug::CloudSlugResolution<'a>> {
    let snapshot = RuntimeCredentialSnapshot::resolve(config).await;
    super::cloud_slug::resolve_cloud_slug_with_access(
        role,
        slug,
        model,
        config,
        extensions,
        CredentialAccess::Runtime(&snapshot),
        None,
    )
}

/// The read-only lookup's secret-free report that an OAuth token is due for
/// a refresh it does not perform.
pub(super) const OAUTH_REFRESH_REQUIRED: &str = "OpenAI OAuth credential requires refresh";

pub(super) struct ReadOnlyCloudCredential {
    pub(super) key: String,
    pub(super) is_oauth: bool,
    pub(super) account_id: Option<String>,
}

pub(super) async fn lookup_key_for_slug_read_only(
    slug: &str,
    config: &Config,
) -> anyhow::Result<ReadOnlyCloudCredential> {
    if slug == crate::config::schema::EPHEMERAL_ROUTE_SLUG {
        if let Some(route) = config.ephemeral_route.as_ref() {
            return Ok(ReadOnlyCloudCredential {
                key: route.api_key.trim().to_string(),
                is_oauth: false,
                account_id: None,
            });
        }
    }

    let auth_profile_key = auth_key_for_slug(slug);
    for provider in [auth_profile_key.as_str(), slug] {
        if let Some((key, is_oauth, expires_at, account_id)) =
            read_only_profile_credential(config, provider, None).await?
        {
            if slug == "openai"
                && is_oauth
                && expires_at.is_some_and(|expires_at| {
                    expires_at <= chrono::Utc::now() + chrono::Duration::minutes(2)
                })
            {
                anyhow::bail!("{OAUTH_REFRESH_REQUIRED}");
            }
            if !key.trim().is_empty() {
                return Ok(ReadOnlyCloudCredential {
                    key,
                    is_oauth,
                    account_id,
                });
            }
        }
    }

    // The OAuth login is stored under a named profile rather than the
    // provider's active API-key profile. Its runtime lookup can refresh and
    // persist the token; doctor only reports when that refresh would be needed.
    if slug == "openai" {
        match read_only_profile_credential(
            config,
            crate::credentials::openai_oauth::OPENAI_PROVIDER_KEY,
            Some(crate::credentials::openai_oauth::OPENAI_OAUTH_PROFILE_NAME),
        )
        .await?
        {
            Some((key, true, expires_at, account_id)) if !key.trim().is_empty() => {
                if expires_at.is_some_and(|expires_at| {
                    expires_at <= chrono::Utc::now() + chrono::Duration::minutes(2)
                }) {
                    anyhow::bail!("{OAUTH_REFRESH_REQUIRED}");
                }
                return Ok(ReadOnlyCloudCredential {
                    key,
                    is_oauth: true,
                    account_id,
                });
            }
            _ => {}
        }
    }

    // Preserve the narrowly scoped legacy config.toml fallback without
    // consulting the mutable auth-profile store.
    if legacy_inference_slug(config) == Some(slug) {
        if let Some(config_key) = config
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            return Ok(ReadOnlyCloudCredential {
                key: config_key.to_string(),
                is_oauth: false,
                account_id: None,
            });
        }
    }

    Ok(ReadOnlyCloudCredential {
        key: String::new(),
        is_oauth: false,
        account_id: None,
    })
}

async fn read_only_profile_credential(
    config: &Config,
    provider: &str,
    profile_override: Option<&str>,
) -> anyhow::Result<Option<ReadOnlyProfileCredential>> {
    let Some(profile) =
        crate::credentials::AuthService::get_profile_read_only(config, provider, profile_override)
            .await
            .map_err(read_only_profile_error)?
    else {
        return Ok(None);
    };

    let (token, is_oauth, expires_at, account_id) = match profile.kind {
        crate::credentials::profiles::AuthProfileKind::Token => {
            (profile.token.unwrap_or_default(), false, None, None)
        }
        crate::credentials::profiles::AuthProfileKind::OAuth => {
            let access_token = profile
                .token_set
                .as_ref()
                .map(|tokens| tokens.access_token.trim().to_string())
                .unwrap_or_default();
            let account_id = profile
                .metadata
                .get("account_id")
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    tinyinference_providers::oauth::openai_account_id_from_access_token(
                        &access_token,
                    )
                });
            (
                access_token,
                true,
                profile.token_set.and_then(|tokens| tokens.expires_at),
                account_id,
            )
        }
    };
    Ok(Some((token, is_oauth, expires_at, account_id)))
}

fn read_only_profile_error(
    error: crate::credentials::profiles::ReadOnlyAuthProfileError,
) -> anyhow::Error {
    match error {
        crate::credentials::profiles::ReadOnlyAuthProfileError::Unavailable => {
            anyhow::anyhow!("read-only credential profile is unavailable")
        }
        crate::credentials::profiles::ReadOnlyAuthProfileError::InvalidStore => {
            anyhow::anyhow!("read-only credential profile store is invalid")
        }
    }
}

/// Auth-profile storage key for a slug-keyed provider.
///
/// New writes use `"provider:<slug>"`. Lookups also try the bare `<slug>`
/// as a legacy fallback (old configs stored keys as e.g. `"openai:default"`).
pub fn auth_key_for_slug(slug: &str) -> String {
    format!("provider:{slug}")
}

/// Redact a configured endpoint to its origin for host diagnostics.
///
/// TinyInference's sanitizer keeps URL paths for routing diagnostics, while
/// OpenHuman endpoints may carry credentials in path segments. Keep this host
/// log policy origin-only and never fall back to the unparsed input.
pub(crate) fn redact_endpoint_origin(endpoint: &str) -> String {
    let Ok(url) = reqwest::Url::parse(endpoint.trim()) else {
        return "<endpoint>".to_string();
    };
    tinyinference_core::sanitize::redact_url(&url.origin().ascii_serialization())
}

/// Whether the openai bearer that [`lookup_key_for_slug`] resolves is an OAuth
/// (Codex-subscription) credential rather than a standard API key.
///
/// OAuth and API-key credentials share the same `provider:openai` profile store
/// and differ only by [`AuthProfileKind`], so the bearer *string* cannot reveal
/// its source — which is exactly why the old `access_token == bearer_key` compare
/// broke under token rotation (#5353). This mirrors `lookup_key_for_slug`'s
/// precedence (`provider:openai`, then the legacy bare `openai`) and reports the
/// *kind* of the profile that would win. With no stored openai profile carrying a
/// credential, the only bearer source is the OAuth fallback, so a present OAuth
/// credential means the bearer is OAuth.
pub(crate) async fn openai_bearer_is_oauth(config: &Config) -> bool {
    use crate::credentials::profiles::AuthProfileKind;

    let auth = AuthService::from_config(config);
    for provider in [auth_key_for_slug("openai"), "openai".to_string()] {
        if let Ok(Some(profile)) = auth.get_profile(&provider, None).await {
            // A profile with an empty credential is skipped by
            // `lookup_key_for_slug`, so fall through to the next precedence level.
            let has_credential = match profile.kind {
                AuthProfileKind::Token => profile
                    .token
                    .as_deref()
                    .is_some_and(|t| !t.trim().is_empty()),
                AuthProfileKind::OAuth => profile
                    .token_set
                    .as_ref()
                    .is_some_and(|t| !t.access_token.trim().is_empty()),
            };
            if has_credential {
                return matches!(profile.kind, AuthProfileKind::OAuth);
            }
        }
    }
    // No stored openai profile with a credential → the bearer, if any, comes from
    // the OAuth fallback (`lookup_openai_bearer_token`).
    crate::credentials::openai_oauth::lookup_openai_oauth_credentials(config)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// Resolve host-selected Codex OAuth metadata through TinyInference's shared
/// routing decision while preserving the configured endpoint for standard
/// requests.
pub(crate) async fn resolve_codex_routing(
    config: &Config,
    slug: &str,
    endpoint: &str,
    bearer_key: &str,
    bearer_is_oauth: bool,
) -> std::result::Result<tinyinference_llm::providers::openai::codex::OpenAiCodexRouting, String> {
    let credentials = if slug == "openai" {
        match crate::credentials::openai_oauth::lookup_openai_oauth_credentials(config).await {
            Ok(credentials) => credentials,
            Err(error) if !bearer_key.trim().is_empty() => {
                log::warn!(
                    "[providers][openai-codex] oauth metadata unavailable; continuing with standard bearer key: {error}"
                );
                None
            }
            Err(error) => {
                return Err(format!(
                    "[chat-factory] openai oauth lookup failed: {error}"
                ));
            }
        }
    } else {
        None
    };
    let use_oauth = bearer_is_oauth && credentials.is_some();
    let account_id = credentials
        .filter(|_| use_oauth)
        .and_then(|credentials| credentials.account_id);
    let mut route = tinyinference_llm::providers::openai::codex::resolve_codex_routing(
        slug,
        endpoint,
        use_oauth,
        account_id.as_deref(),
    );
    if !route.using_oauth {
        route.endpoint = endpoint.to_string();
    }
    Ok(route)
}

/// Fetch the bearer token for a slug from the workspace `auth-profiles.json`.
///
/// Tries `provider:<slug>` first (new key format), then the bare `<slug>`
/// (legacy format where keys were stored as `"openai"`, `"anthropic"`, etc.).
/// Missing or empty keys return `Ok(String::new())` — callers treat that as
/// "no auth", which surfaces an authentication error at first call rather than
/// at factory build time.
pub async fn lookup_key_for_slug(slug: &str, config: &Config) -> anyhow::Result<String> {
    // Ahead of the stored profiles, and scoped to the one slug the per-call
    // route registers. A caller that named an endpoint and a bearer for this
    // turn has said where the credential comes from, and there is nothing on
    // disk to find for a slug that exists only in this `Config` copy. Scoping it
    // by slug is what keeps the bearer from reaching a provider the caller never
    // named — the same containment the legacy `config.api_key` fallback below
    // gets from `legacy_inference_slug`.
    if slug == crate::config::schema::EPHEMERAL_ROUTE_SLUG {
        if let Some(route) = config.ephemeral_route.as_ref() {
            log::debug!(
                "[providers][chat-factory] auth lookup slug={} key_present={} (per-call route)",
                slug,
                !route.api_key.trim().is_empty()
            );
            return Ok(route.api_key.trim().to_string());
        }
    }

    let auth = AuthService::from_config(config);
    // Try new-style key first.
    let new_key = auth_key_for_slug(slug);
    if let Ok(Some(k)) = auth.get_provider_bearer_token(&new_key, None).await {
        if !k.is_empty() {
            log::debug!(
                "[providers][chat-factory] auth lookup slug={} key_present=true (new-style)",
                slug
            );
            return Ok(k);
        }
    }
    // Fall back to legacy bare slug.
    let key = auth
        .get_provider_bearer_token(slug, None)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "[chat-factory] failed to read API key for slug '{}': {}",
                slug,
                e
            )
        })?
        .unwrap_or_default();
    if !key.is_empty() {
        log::debug!(
            "[providers][chat-factory] auth lookup slug={} key_present=true",
            slug
        );
        return Ok(key);
    }

    // OAuth fallback for `openai` runs only after standard API-key resolution
    // returns empty, so env/audit/metrics in the standard path always execute
    // and the OAuth path never silently bypasses provider-agnostic logic.
    if slug == "openai" {
        match crate::credentials::openai_oauth::lookup_openai_bearer_token(config).await {
            Ok(Some(token)) if !token.is_empty() => {
                log::debug!(
                    "[providers][chat-factory] auth lookup slug={} key_present=true (oauth)",
                    slug
                );
                return Ok(token);
            }
            Ok(_) => {}
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "[chat-factory] openai oauth lookup failed: {e}"
                ));
            }
        }
    }

    // Fallback: read from top-level config.api_key (direct config.toml api_key).
    // This handles the case where a key was set in config.toml but not saved
    // through the UI into auth-profiles.json.
    //
    // Scoped to the legacy direct-inference provider only — the cloud-provider
    // slug whose endpoint matches `config.inference_url`. `config.api_key` was
    // historically paired with `inference_url` for direct endpoint routing, so
    // an unscoped fallback would leak this global key to any other provider
    // whose auth-profile lookup returned empty (cross-provider credential leak
    // flagged by CodeRabbit + maintainers on #2724).
    if legacy_inference_slug(config) == Some(slug) {
        if let Some(config_key) = config.api_key.as_ref() {
            if !config_key.trim().is_empty() {
                log::debug!(
                    "[providers][chat-factory] auth lookup slug={} key_present=true (config.toml fallback for legacy inference_url)",
                    slug
                );
                return Ok(config_key.trim().to_string());
            }
        }
    }

    log::debug!(
        "[providers][chat-factory] auth lookup slug={} key_present=false",
        slug
    );
    Ok(String::new())
}
