//! Cloud provider credential schema.
//!
//! Each entry in `Config::cloud_providers` represents one configured LLM
//! backend. Providers are keyed by a user-chosen `slug` (e.g. `"openai"`,
//! `"my-deepseek"`). The factory in `inference::provider::factory`
//! resolves workload-to-provider strings against this list at runtime using
//! the grammar `"<slug>:<model>"`.
//!
//! Legacy provider discriminators are decoded by the schema-14 config migration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use std::sync::LazyLock;

use tinyinference_providers::builtin::{
    BuiltinAuthStyle, BUILTIN_CLOUD_PROVIDERS as UPSTREAM_BUILTINS,
};

/// A built-in cloud provider preset in the host's [`AuthStyle`] vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinCloudProvider {
    pub slug: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub auth_style: AuthStyle,
}

/// The managed OpenHuman route. It is the one preset `tinyinference-providers`
/// deliberately omits: managed gateway policy is host-owned.
const OPENHUMAN_PRESET: BuiltinCloudProvider = BuiltinCloudProvider {
    slug: "openhuman",
    label: "OpenHuman",
    endpoint: "https://api.openhuman.ai/v1",
    auth_style: AuthStyle::OpenhumanJwt,
};

impl From<&'static tinyinference_providers::builtin::BuiltinCloudProvider>
    for BuiltinCloudProvider
{
    fn from(preset: &'static tinyinference_providers::builtin::BuiltinCloudProvider) -> Self {
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

/// Every built-in preset: the managed OpenHuman route followed by the
/// provider-neutral catalog from `tinyinference-providers`
/// ([`UPSTREAM_BUILTINS`]), mapped onto the host's [`AuthStyle`].
pub static BUILTIN_CLOUD_PROVIDERS: LazyLock<Vec<BuiltinCloudProvider>> = LazyLock::new(|| {
    std::iter::once(OPENHUMAN_PRESET)
        .chain(UPSTREAM_BUILTINS.iter().map(BuiltinCloudProvider::from))
        .collect()
});

pub(crate) fn builtin_cloud_provider(type_str: &str) -> Option<&'static BuiltinCloudProvider> {
    BUILTIN_CLOUD_PROVIDERS
        .iter()
        .find(|provider| provider.slug == type_str)
}

/// Whether `slug` matches a built-in cloud provider preset.
///
/// The chat factory uses this to decide capability defaults (e.g. whether the
/// provider exposes the OpenAI Responses API) only for providers we ship and
/// therefore know the API surface of. Custom / user-defined slugs are treated
/// as unknown and keep the permissive defaults.
pub fn is_builtin_cloud_slug(slug: &str) -> bool {
    builtin_cloud_provider(slug).is_some()
}

/// Whether a built-in cloud provider exposes the OpenAI **Responses API**
/// (`/v1/responses`).
///
/// Only OpenAI's first-party endpoint serves `/responses`; every other built-in
/// preset (DeepSeek, Groq, Mistral, Fireworks, …) is chat-completions-only.
/// Enabling the chat-completions-404 → `/responses` fallback for those
/// guarantees a second 404 against an endpoint that does not exist, which floods
/// Sentry with an empty-body `"<provider> Responses API error:"` event
/// (TAURI-RUST-5EN — same class as the local-provider TAURI-RUST-59Y fix). The
/// factory consults this to build chat-completions-only built-ins with
/// `new_no_responses_fallback`.
///
/// Custom / unknown slugs are intentionally NOT covered here (see
/// [`is_builtin_cloud_slug`]): a user-defined OpenAI-compatible endpoint may be
/// a genuine OpenAI proxy that does support `/responses`, so the factory keeps
/// the fallback for those.
pub fn builtin_cloud_supports_responses_api(slug: &str) -> bool {
    matches!(slug, "openai")
}

/// Extract the lowercased authority host from an endpoint URL, dropping the
/// scheme, any userinfo, the port, and the path. Returns `None` when no host
/// can be parsed. Tolerant of a missing scheme and of IPv6 literals.
pub fn endpoint_host(endpoint: &str) -> Option<String> {
    let s = endpoint.trim();
    // Drop the scheme (`https://…`); tolerate a bare `host/path` form.
    let after_scheme = s.split_once("://").map(|(_, rest)| rest).unwrap_or(s);
    // The authority ends at the first path / query / fragment delimiter.
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Strip any `user:pass@` userinfo prefix.
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    // Strip the port, handling bracketed IPv6 literals (`[::1]:8080`).
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split_once(']').map(|(h, _)| h).unwrap_or(rest)
    } else {
        host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port)
    };
    let host = host.trim().to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Whether `host` is the authority host of any built-in cloud **inference**
/// provider (e.g. `openrouter.ai`, `api.openai.com`, `api.groq.com`).
///
/// Derived entirely from [`BUILTIN_CLOUD_PROVIDERS`] so the set stays in sync
/// with the provider registry. `host` is compared case-insensitively against
/// each preset's [`endpoint_host`].
///
/// # Why this exists
///
/// `config.api_url` is overloaded: it is the chat/inference endpoint, but
/// `api::config::effective_backend_api_url` also reuses it as the
/// OpenHuman **backend** base for team/billing/auth calls. A BYO user who
/// points `api_url` at a provider's canonical base (`https://openrouter.ai/api/v1`)
/// would otherwise have every backend domain call routed to the inference host
/// → 400/404 (TAURI-RUST-HW1: 4932 team-usage 400s from `openrouter.ai`).
/// The backend-URL resolver uses this to treat such hosts as non-backend and
/// fall back to the default backend chain — the cloud analogue of the local-AI
/// guard that fixed the Ollama case (OPENHUMAN-TAURI-51/-80/-7Z).
pub fn host_is_builtin_cloud_provider(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    BUILTIN_CLOUD_PROVIDERS
        .iter()
        .any(|p| endpoint_host(p.endpoint).as_deref() == Some(host.as_str()))
}

/// Whether an endpoint **host** is a known cloud host that does NOT serve the
/// OpenAI Responses API (`/v1/responses`) — i.e. it is chat-completions-only,
/// regardless of which user slug points at it.
///
/// Derived entirely from [`BUILTIN_CLOUD_PROVIDERS`]: a host is chat-only when
/// some built-in preset uses it AND no preset at that host advertises the
/// Responses API (only OpenAI's `api.openai.com` does). This closes the
/// custom-slug gap behind the builtin-slug gate
/// ([`builtin_cloud_supports_responses_api`]): a user slug pointed at, e.g.,
/// `integrate.api.nvidia.com` must never attempt `/responses` (TAURI-RUST-5A1),
/// while a genuinely unknown proxy host keeps the permissive fallback so a real
/// OpenAI proxy still gets `/responses`.
pub fn endpoint_host_is_chat_completions_only(endpoint: &str) -> bool {
    let Some(host) = endpoint_host(endpoint) else {
        return false;
    };
    let mut matched_chat_only = false;
    for provider in BUILTIN_CLOUD_PROVIDERS.iter() {
        if endpoint_host(provider.endpoint).as_deref() == Some(host.as_str()) {
            if builtin_cloud_supports_responses_api(provider.slug) {
                // A Responses-capable built-in lives at this host → not chat-only.
                return false;
            }
            matched_chat_only = true;
        }
    }
    matched_chat_only
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
        }
    }
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
/// `inference::provider::ops::list_configured_models`
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
    use std::time::{SystemTime, UNIX_EPOCH};
    // Cheap pseudo-random from timestamp nanoseconds — adequate for local
    // config uniqueness without pulling in a PRNG crate.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut suffix = String::with_capacity(5);
    let mut seed = nanos as usize;
    for _ in 0..5 {
        suffix.push(chars[seed % chars.len()] as char);
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed = (seed >> 33) ^ seed;
    }
    // Sanitise slug to only alphanumeric + '-' for the id prefix.
    let safe_slug: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(20)
        .collect();
    format!("p_{}_{}", safe_slug, suffix)
}

#[cfg(test)]
#[path = "cloud_providers_tests.rs"]
mod tests;
