//! Cloud provider credential schema.
//!
//! Each entry in `Config::cloud_providers` represents one configured LLM
//! backend. Providers are keyed by a user-chosen `slug` (e.g. `"openai"`,
//! `"my-deepseek"`). Inference routing in `config::ops::primary_cloud`
//! resolves workload-to-provider strings against this list at runtime using
//! the grammar `"<slug>:<model>"`.
//!
//! Legacy provider discriminators are decoded by the schema-14 config migration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::slots::HookSlot;
use tinyinference_providers::builtin::{
    BuiltinAuthStyle, BuiltinCloudProvider as UpstreamBuiltinCloudProvider, BuiltinPresets,
    BUILTIN_CLOUD_PROVIDERS as UPSTREAM_BUILTINS,
};

static HOSTED_PRESETS: HookSlot<BuiltinCloudProvider> =
    HookSlot::new("config::hosted_cloud_presets");
pub type HostedCloudIdentityClassifier = Arc<dyn Fn(&str) -> bool + Send + Sync>;
pub type HostedAuthStyleClassifier = Arc<dyn Fn(AuthStyle) -> bool + Send + Sync>;
static HOSTED_CLOUD_IDENTITY: HookSlot<HostedCloudIdentityClassifier> =
    HookSlot::new("config::hosted_cloud_identity");
static HOSTED_AUTH_STYLE: HookSlot<HostedAuthStyleClassifier> =
    HookSlot::new("config::hosted_auth_style");

/// A built-in cloud provider preset in the host's [`AuthStyle`] vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinCloudProvider {
    pub slug: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub auth_style: AuthStyle,
}

/// Register an additional cloud preset supplied by a host.
pub fn register_hosted_cloud_preset(id: impl Into<String>, preset: BuiltinCloudProvider) {
    HOSTED_PRESETS.register(id, preset);
}

/// Register a host preset for one runtime context.
pub fn register_hosted_cloud_preset_in(
    extensions: &crate::core::slots::Extensions,
    id: impl Into<String>,
    preset: BuiltinCloudProvider,
) {
    HOSTED_PRESETS.register_in(extensions, id, preset);
}

/// Register host-owned auth-style classification without making it core
/// inference policy.
pub fn register_hosted_cloud_provider_policy(
    identity: HostedCloudIdentityClassifier,
    auth_style: HostedAuthStyleClassifier,
) {
    HOSTED_CLOUD_IDENTITY.register("default", identity);
    HOSTED_AUTH_STYLE.register("default", auth_style);
}

/// Register hosted-provider classification for one runtime context.
pub fn register_hosted_cloud_provider_policy_in(
    extensions: &crate::core::slots::Extensions,
    identity: HostedCloudIdentityClassifier,
    auth_style: HostedAuthStyleClassifier,
) {
    HOSTED_CLOUD_IDENTITY.register_in(extensions, "default", identity);
    HOSTED_AUTH_STYLE.register_in(extensions, "default", auth_style);
}

/// Whether the installed host owns this reserved cloud-provider identity.
pub fn is_hosted_cloud_identity(slug: &str) -> bool {
    HOSTED_CLOUD_IDENTITY
        .get("default")
        .is_some_and(|classify| classify(slug))
}

/// Whether the installed host owns this passive compatibility auth style.
pub fn is_hosted_auth_style(style: AuthStyle) -> bool {
    HOSTED_AUTH_STYLE
        .get("default")
        .is_some_and(|classify| classify(style))
}

/// Whether the selected runtime or process host owns this compatibility auth style.
pub(crate) fn is_hosted_auth_style_for(
    extensions: Option<&crate::core::slots::Extensions>,
    style: AuthStyle,
) -> bool {
    HOSTED_AUTH_STYLE
        .get_for(extensions, "default")
        .is_some_and(|classify| classify(style))
}

impl From<&tinyinference_providers::builtin::BuiltinCloudProvider> for BuiltinCloudProvider {
    fn from(preset: &tinyinference_providers::builtin::BuiltinCloudProvider) -> Self {
        Self {
            slug: preset.slug,
            label: preset.label,
            endpoint: preset.endpoint,
            auth_style: match preset.auth_style {
                BuiltinAuthStyle::Bearer => AuthStyle::Bearer,
                BuiltinAuthStyle::Anthropic => AuthStyle::Anthropic,
            },
        }
    }
}

/// Every built-in preset: host-provided entries followed by the
/// provider-neutral catalog from `tinyinference-providers`.
pub fn builtin_cloud_providers() -> Vec<BuiltinCloudProvider> {
    let hosted = HOSTED_PRESETS
        .entries_context_first()
        .into_iter()
        .map(|(_, preset)| preset)
        .collect::<Vec<_>>();
    // Keep the product's managed entry first for settings presentation. The
    // owner catalog itself remains generic-first for exact-slug lookup.
    hosted
        .iter()
        .copied()
        .filter(|preset| {
            !UPSTREAM_BUILTINS
                .iter()
                .any(|generic| generic.slug == preset.slug)
        })
        .chain(UPSTREAM_BUILTINS.iter().map(BuiltinCloudProvider::from))
        .collect::<Vec<_>>()
}

/// Generic provider catalog extended with the host's registered preset
/// metadata. Host auth policy remains in the owning host registry.
pub fn builtin_presets() -> BuiltinPresets {
    let hosted = HOSTED_PRESETS
        .entries_context_first()
        .into_iter()
        .map(|(_, preset)| preset)
        .collect::<Vec<_>>();
    BuiltinPresets::new(hosted.iter().map(|preset| UpstreamBuiltinCloudProvider {
        slug: preset.slug,
        label: preset.label,
        endpoint: preset.endpoint,
        // Only route/catalog metadata is consumed from this generic catalog;
        // persisted OpenhumanJwt remains in the host's schema and is never
        // converted into runtime bearer authentication.
        auth_style: match preset.auth_style {
            AuthStyle::Anthropic => BuiltinAuthStyle::Anthropic,
            AuthStyle::Bearer | AuthStyle::OpenhumanJwt | AuthStyle::None => {
                BuiltinAuthStyle::Bearer
            }
        },
    }))
}

/// Looks up a provider preset, giving the generic catalog precedence over
/// host additions with the same slug.
pub fn builtin_cloud_provider(type_str: &str) -> Option<BuiltinCloudProvider> {
    if let Some(preset) = BuiltinPresets::default().builtin_cloud_provider(type_str) {
        return Some(BuiltinCloudProvider::from(preset));
    }
    let hosted = HOSTED_PRESETS
        .entries_context_first()
        .into_iter()
        .map(|(_, preset)| preset)
        .collect::<Vec<_>>();
    if let Some(preset) = hosted.iter().find(|preset| preset.slug == type_str) {
        return Some(*preset);
    }
    None
}

/// Authentication header style for a cloud provider.
///
/// Wire format is lowercase (e.g. `"bearer"`). Determines which HTTP headers
/// are attached when calling the provider's API.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthStyle {
    /// OpenAI-compatible: `Authorization: Bearer <key>`
    #[default]
    Bearer,
    /// Anthropic: `x-api-key: <key>` + `anthropic-version: 2023-06-01`
    Anthropic,
    /// OpenHuman session JWT (injected by the backend provider, not stored here).
    OpenhumanJwt,
    /// No auth header — e.g. local Ollama.
    None,
}

impl AuthStyle {
    /// Parse the RPC compatibility spellings for a provider authentication style.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "bearer" => Ok(Self::Bearer),
            "anthropic" => Ok(Self::Anthropic),
            "openhuman_jwt" | "openhumanjwt" => Ok(Self::OpenhumanJwt),
            "none" => Ok(Self::None),
            other => Err(format!(
                "unknown auth_style '{}'; valid: bearer, anthropic, openhuman_jwt, none",
                other
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::Anthropic => "anthropic",
            Self::OpenhumanJwt => "openhuman_jwt",
            Self::None => "none",
        }
    }
}

/// Endpoint config for one cloud LLM provider.
///
/// **Note on secrets**: API keys are NOT stored on this struct. They live in
/// `auth-profiles.json` via `security::credentials::AuthService`,
/// keyed by `provider:<slug>` (falling back to bare `<slug>` for legacy
/// entries). The factory looks up the token at call time via
/// `inference::provider::factory::auth_key_for_slug`.
///
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct CloudProviderCreds {
    /// Opaque stable id, e.g. `"p_openai_a8c3f"`. Never shown in the UI.
    /// Generated once by [`generate_provider_id`] and never changes.
    pub id: String,
    /// Routing key chosen by the user or seeded from the legacy type.
    /// Lower-case alphanumeric + `-`. Must be unique per config and not in the
    /// reserved list (see [`is_slug_reserved`]). The factory resolves
    /// `"<slug>:<model>"` strings against this field.
    pub slug: String,
    /// Human-readable display label, supplied by the frontend. Not used in routing.
    pub label: String,
    /// OpenAI-compatible base URL (`/models`, `/chat/completions` etc. are appended).
    pub endpoint: String,
    /// Authentication header style.
    pub auth_style: AuthStyle,

    /// Default provider-native model used by BYOK routes without a model suffix.
    // keep until v0.66 (review after 2027-01-11): active BYOK routes and the
    // inference settings RPC depend on this field; replacement needs route migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,

    /// Extra HTTP headers sent on every request to this provider (chat,
    /// completions and model listing), on top of the auth headers
    /// [`auth_style`](Self::auth_style) adds. The way to satisfy a provider
    /// that requires a header the auth style does not cover, without waiting
    /// for a release:
    ///
    /// ```toml
    /// [[cloud_providers]]
    /// slug = "my-gateway"
    /// endpoint = "https://gateway.example/v1"
    /// auth_style = "bearer"
    ///
    /// [cloud_providers.headers]
    /// x-tenant = "team-a"
    /// ```
    ///
    /// Names are matched case-insensitively against the headers OpenHuman adds
    /// itself (`User-Agent`, `x-opencode-session`), and a configured value
    /// replaces the built-in one. Values are sent verbatim and may be secrets,
    /// so they never reach `info`-level logs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl Default for CloudProviderCreds {
    fn default() -> Self {
        Self {
            id: String::new(),
            slug: String::new(),
            label: String::new(),
            endpoint: String::new(),
            auth_style: AuthStyle::Bearer,
            default_model: None,
            headers: BTreeMap::new(),
        }
    }
}

/// What a client-facing config snapshot shows in place of a configured
/// header value. Header values may be secrets, so only their names leave the
/// core; a settings save that echoes this placeholder back keeps the stored
/// value (see `config::ops::apply_model_settings`).
pub const REDACTED_HEADER_VALUE: &str = "[REDACTED]";

/// `headers` with every value replaced by [`REDACTED_HEADER_VALUE`].
pub fn redacted_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .keys()
        .map(|name| (name.clone(), REDACTED_HEADER_VALUE.to_string()))
        .collect()
}

/// Checks that every configured header on the provider `slug` is a valid
/// HTTP header name and value, so a typo in `config.toml` or a settings save
/// fails with an error that names the provider and the header instead of an
/// opaque request-builder error on every call. The value itself is never part
/// of the error: it may be a secret.
pub fn validate_provider_headers(
    slug: &str,
    headers: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (name, value) in headers {
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("cloud provider '{slug}': invalid header name '{name}'"))?;
        reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| format!("cloud provider '{slug}': invalid value for header '{name}'"))?;
    }
    Ok(())
}

/// Reserved slugs that may not be used for user-configured providers.
/// These are sentinels in the factory's routing grammar.
///
/// `ollama` is deliberately NOT reserved: the AI settings panel registers an
/// `ollama` `cloud_providers` entry so `list_configured_models` can resolve
/// the user's chosen base_url for the model dropdown. The factory's chat
/// routing is unaffected — the `ollama:<model>` prefix branch in
/// `factory::create_chat_provider_from_string` fires before the
/// `<slug>:<model>` cloud-provider lookup, so a synthetic `ollama` entry
/// never reaches `make_cloud_provider_by_slug`. When no `cloud_providers`
/// row exists (config drift, upgrade from a build that only persisted
/// `config.local_ai.base_url`, flush-vs-probe race),
/// `inference::provider::models::catalog_listing::list_configured_models`
/// falls back to a synthetic entry via `synthesize_local_runtime_entry`
/// (Sentry TAURI-RUST-28Z fix). The same fallback applies to `lmstudio`.
pub fn is_slug_reserved(s: &str) -> bool {
    matches!(s.trim(), "" | "cloud" | "openhuman" | "pid")
}

/// Generate a short opaque id for a new provider entry.
///
/// Format: `"p_<slug>_<5 random alphanumerics>"`, e.g. `"p_openai_a8c3f"`.
/// The random suffix is not cryptographically strong — it only needs to be
/// unique within a single user's config file.
pub fn generate_provider_id(slug: &str) -> String {
    super::generate_provider_id_with_prefix("p", slug)
}

#[cfg(test)]
#[path = "cloud_providers_tests.rs"]
mod tests;
