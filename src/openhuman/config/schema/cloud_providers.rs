//! Cloud provider credential schema.
//!
//! Each entry in `Config::cloud_providers` represents one configured LLM
//! backend. Providers are keyed by a user-chosen `slug` (e.g. `"openai"`,
//! `"my-deepseek"`). The factory in `crate::openhuman::inference::provider::factory`
//! resolves workload-to-provider strings against this list at runtime using
//! the grammar `"<slug>:<model>"`.
//!
//! Legacy configs that use `type`/`default_model` are migrated in-memory on
//! load via `migrate_legacy_fields()`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinCloudProvider {
    pub slug: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub auth_style: AuthStyle,
}

pub const BUILTIN_CLOUD_PROVIDERS: &[BuiltinCloudProvider] = &[
    BuiltinCloudProvider {
        slug: "openhuman",
        label: "OpenHuman",
        endpoint: "https://api.openhuman.ai/v1",
        auth_style: AuthStyle::OpenhumanJwt,
    },
    BuiltinCloudProvider {
        slug: "openai",
        label: "OpenAI",
        endpoint: "https://api.openai.com/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "anthropic",
        label: "Anthropic",
        endpoint: "https://api.anthropic.com/v1",
        auth_style: AuthStyle::Anthropic,
    },
    BuiltinCloudProvider {
        slug: "openrouter",
        label: "OpenRouter",
        endpoint: "https://openrouter.ai/api/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "orcarouter",
        label: "OrcaRouter",
        endpoint: "https://api.orcarouter.ai/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "gmi",
        label: "GMI",
        endpoint: "https://api.gmi-serving.com/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "fireworks",
        label: "Fireworks",
        endpoint: "https://api.fireworks.ai/inference/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "moonshot",
        label: "Kimi (Moonshot)",
        endpoint: "https://api.moonshot.ai/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "groq",
        label: "Groq",
        endpoint: "https://api.groq.com/openai/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "mistral",
        label: "Mistral",
        endpoint: "https://api.mistral.ai/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "deepseek",
        label: "DeepSeek",
        endpoint: "https://api.deepseek.com/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "together",
        label: "Together AI",
        endpoint: "https://api.together.xyz/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "google",
        label: "Google Gemini",
        endpoint: "https://generativelanguage.googleapis.com/v1beta/openai",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "cerebras",
        label: "Cerebras",
        endpoint: "https://api.cerebras.ai/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "xai",
        label: "xAI",
        endpoint: "https://api.x.ai/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "huggingface",
        label: "Hugging Face",
        endpoint: "https://router.huggingface.co/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "nvidia",
        label: "NVIDIA",
        endpoint: "https://integrate.api.nvidia.com/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "zai",
        label: "Z.AI",
        endpoint: "https://api.z.ai/api/paas/v4",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "minimax",
        label: "MiniMax",
        // MiniMax exposes a full OpenAI-compatible surface at `/v1`
        // (`/v1/chat/completions`, `/v1/models`). The previous `/anthropic`
        // base + Anthropic auth pointed at MiniMax's Messages-protocol API,
        // which OpenHuman does not speak — it only builds OpenAI-style
        // `/chat/completions` and `/models` — so both chat and model-listing
        // 404'd (`/anthropic/chat/completions`, `/anthropic/models`). The
        // 404 on model-listing was Sentry TAURI-RUST-8X3. Use the `/v1`
        // OpenAI surface with Bearer auth so both paths resolve.
        endpoint: "https://api.minimax.io/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "stepfun",
        label: "StepFun",
        endpoint: "https://api.stepfun.ai/step_plan/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "kilocode",
        label: "Kilo Code",
        endpoint: "https://api.kilo.ai/api/gateway",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "deepinfra",
        label: "DeepInfra",
        endpoint: "https://api.deepinfra.com/v1/openai",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "novita",
        label: "Novita",
        endpoint: "https://api.novita.ai/v3/openai",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "venice",
        label: "Venice",
        endpoint: "https://api.venice.ai/api/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "vercel-ai-gateway",
        label: "Vercel AI Gateway",
        endpoint: "https://ai-gateway.vercel.sh/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "sumopod",
        label: "SumoPod",
        endpoint: "https://ai.sumopod.com/v1",
        auth_style: AuthStyle::Bearer,
    },
    BuiltinCloudProvider {
        slug: "modelscope",
        label: "ModelScope",
        endpoint: "https://api-inference.modelscope.cn/v1",
        auth_style: AuthStyle::Bearer,
    },
];

fn builtin_cloud_provider(type_str: &str) -> Option<&'static BuiltinCloudProvider> {
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
pub(crate) fn endpoint_host(endpoint: &str) -> Option<String> {
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
/// [`crate::api::config::effective_backend_api_url`] also reuses it as the
/// OpenHuman **backend** base for team/billing/auth calls. A BYO user who
/// points `api_url` at a provider's canonical base (`https://openrouter.ai/api/v1`)
/// would otherwise have every backend domain call routed to the inference host
/// → 400/404 (TAURI-RUST-HW1: 4932 `GET /teams/me/usage` 400s from `openrouter.ai`).
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
    for provider in BUILTIN_CLOUD_PROVIDERS {
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
/// `auth-profiles.json` via [`crate::openhuman::credentials::AuthService`],
/// keyed by `provider:<slug>` (falling back to bare `<slug>` for legacy
/// entries). The factory looks up the token at call time via
/// [`crate::openhuman::inference::provider::factory::auth_key_for_slug`].
///
/// ## Back-compat
///
/// Old configs may have `type` and `default_model` fields. These are
/// tolerated on read (via `legacy_type` / `default_model`) but never written.
/// Call `migrate_legacy_fields()` after deserialising.
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

    // ── Back-compat: old `type` field ───────────────────────────────────────
    /// Legacy discriminator written by older builds. Read-only; never emitted.
    #[serde(rename = "type", default, skip_serializing)]
    pub legacy_type: Option<String>,

    // ── Back-compat: old `default_model` field ──────────────────────────────
    /// Legacy default model written by older builds. Read-only; never emitted.
    #[serde(default, skip_serializing)]
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
            legacy_type: None,
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
/// [`crate::openhuman::inference::provider::ops::list_configured_models`]
/// falls back to a synthetic entry via `synthesize_local_runtime_entry`
/// (Sentry TAURI-RUST-28Z fix). The same fallback applies to `lmstudio`.
pub fn is_slug_reserved(s: &str) -> bool {
    matches!(s.trim(), "" | "cloud" | "openhuman" | "pid")
}

/// Apply legacy field migration in-place.
///
/// Idempotent: only fills in empty fields from the legacy `type`/`default_model`
/// values. Safe to call on already-migrated entries.
pub fn migrate_legacy_fields(entry: &mut CloudProviderCreds) {
    let legacy_type = entry.legacy_type.clone().unwrap_or_default();
    let lt = legacy_type.trim();

    // Slug from legacy type when missing.
    if entry.slug.is_empty() && !lt.is_empty() {
        entry.slug = lt.to_string();
        log::debug!(
            "[config][cloud_providers] migrated slug from legacy type='{}' id={}",
            lt,
            entry.id
        );
    }

    // Label from static map when missing.
    if entry.label.is_empty() {
        entry.label = legacy_label_for(if entry.slug.is_empty() {
            lt
        } else {
            &entry.slug
        })
        .to_string();
        log::debug!(
            "[config][cloud_providers] migrated label='{}' for slug='{}' id={}",
            entry.label,
            entry.slug,
            entry.id
        );
    }

    // Endpoint from legacy defaults when missing.
    if entry.endpoint.is_empty() {
        let ep = legacy_default_endpoint(lt);
        if !ep.is_empty() {
            entry.endpoint = ep.to_string();
        }
    }

    // Auth style from legacy type when still at default Bearer.
    if entry.auth_style == AuthStyle::Bearer {
        if let Some(provider) = builtin_cloud_provider(lt) {
            entry.auth_style = provider.auth_style;
        }
    }
}

/// Map a legacy type string (or slug) to a human-readable label.
fn legacy_label_for(type_str: &str) -> &'static str {
    builtin_cloud_provider(type_str)
        .map(|provider| provider.label)
        .unwrap_or("Custom")
}

/// Map a legacy type string to its well-known default endpoint.
fn legacy_default_endpoint(type_str: &str) -> &'static str {
    builtin_cloud_provider(type_str)
        .map(|provider| provider.endpoint)
        .unwrap_or("")
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

// ── Back-compat type alias ──────────────────────────────────────────────────
// Kept so existing code that imports `CloudProviderType` compiles without
// sweeping changes. New code should use `AuthStyle` directly.

/// Legacy discriminator enum. **Deprecated**: use `AuthStyle` on new entries.
/// Retained only to satisfy callers that still pattern-match on
/// `CloudProviderType` (e.g. the migration module). Will be removed once all
/// call sites are updated to slug-keyed lookups.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CloudProviderType {
    Openhuman,
    Openai,
    Anthropic,
    Openrouter,
    Orcarouter,
    Custom,
}

impl CloudProviderType {
    /// Well-known default base URL for each provider type.
    pub fn default_endpoint(&self) -> &'static str {
        match self {
            Self::Openhuman => "https://api.openhuman.ai/v1",
            Self::Openai => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::Openrouter => "https://openrouter.ai/api/v1",
            Self::Orcarouter => "https://api.orcarouter.ai/v1",
            Self::Custom => "",
        }
    }

    /// Human-readable label used in logs and error messages.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Openhuman => "OpenHuman",
            Self::Openai => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Openrouter => "OpenRouter",
            Self::Orcarouter => "OrcaRouter",
            Self::Custom => "Custom",
        }
    }

    /// Lowercase wire-format string (matches JSON serialisation).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Openhuman => "openhuman",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Openrouter => "openrouter",
            Self::Orcarouter => "orcarouter",
            Self::Custom => "custom",
        }
    }

    /// Corresponding `AuthStyle`.
    pub fn auth_style(&self) -> AuthStyle {
        match self {
            Self::Openhuman => AuthStyle::OpenhumanJwt,
            Self::Anthropic => AuthStyle::Anthropic,
            _ => AuthStyle::Bearer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        builtin_cloud_supports_responses_api, endpoint_host,
        endpoint_host_is_chat_completions_only, host_is_builtin_cloud_provider,
        is_builtin_cloud_slug, is_slug_reserved, migrate_legacy_fields, AuthStyle,
        CloudProviderCreds, BUILTIN_CLOUD_PROVIDERS,
    };

    #[test]
    fn reserved_slugs() {
        for s in ["", " ", "cloud", "openhuman", "pid"] {
            assert!(is_slug_reserved(s), "{s:?} must stay reserved");
        }
    }

    // Regression: `ollama` was previously reserved, which made the AI settings
    // panel unable to persist an `ollama` cloud_providers entry — so the
    // model-list dropdown failed with "no cloud provider with id or slug
    // 'ollama' found". The factory's chat routing is unaffected by this
    // change because the `ollama:<model>` prefix branch fires before any
    // cloud_providers lookup.
    #[test]
    fn ollama_and_lmstudio_are_not_reserved() {
        assert!(
            !is_slug_reserved("ollama"),
            "ollama must be usable as a cloud_providers slug for the /models probe"
        );
        assert!(
            !is_slug_reserved("lmstudio"),
            "lmstudio is a free-form OpenAI-compatible slug"
        );
    }

    #[test]
    fn builtin_cloud_provider_defaults_cover_phase_one_presets() {
        for (slug, label, endpoint, auth_style) in [
            (
                "groq",
                "Groq",
                "https://api.groq.com/openai/v1",
                AuthStyle::Bearer,
            ),
            (
                "deepseek",
                "DeepSeek",
                "https://api.deepseek.com/v1",
                AuthStyle::Bearer,
            ),
            (
                "minimax",
                "MiniMax",
                "https://api.minimax.io/v1",
                AuthStyle::Bearer,
            ),
            (
                "sumopod",
                "SumoPod",
                "https://ai.sumopod.com/v1",
                AuthStyle::Bearer,
            ),
            (
                "modelscope",
                "ModelScope",
                "https://api-inference.modelscope.cn/v1",
                AuthStyle::Bearer,
            ),
        ] {
            let mut entry = CloudProviderCreds {
                id: format!("p_{slug}"),
                legacy_type: Some(slug.to_string()),
                ..Default::default()
            };
            migrate_legacy_fields(&mut entry);

            assert_eq!(entry.slug, slug);
            assert_eq!(entry.label, label);
            assert_eq!(entry.endpoint, endpoint);
            assert_eq!(entry.auth_style, auth_style);
        }
    }

    #[test]
    fn builtin_cloud_provider_slugs_are_unique() {
        let mut slugs = std::collections::HashSet::new();
        for provider in BUILTIN_CLOUD_PROVIDERS {
            assert!(
                slugs.insert(provider.slug),
                "duplicate built-in cloud provider slug {}",
                provider.slug
            );
        }
    }

    #[test]
    fn is_builtin_cloud_slug_matches_presets_only() {
        for slug in ["openai", "deepseek", "groq", "mistral"] {
            assert!(is_builtin_cloud_slug(slug), "{slug} is a built-in preset");
        }
        for slug in ["my-proxy", "custom-openai", "totally-unknown", ""] {
            assert!(
                !is_builtin_cloud_slug(slug),
                "{slug:?} is not a built-in preset"
            );
        }
    }

    #[test]
    fn only_openai_builtin_exposes_responses_api() {
        assert!(builtin_cloud_supports_responses_api("openai"));
        for slug in ["deepseek", "groq", "mistral", "fireworks", "together"] {
            assert!(
                !builtin_cloud_supports_responses_api(slug),
                "{slug} is chat-completions-only and must not advertise the Responses API"
            );
        }
    }

    /// Drift guard (TAURI-RUST-5EN): couple the capability helper to the
    /// preset list so adding a new built-in that wrongly claims the Responses
    /// API — or renaming `openai` — fails CI rather than silently re-enabling
    /// the guaranteed-404 `/responses` fallback. OpenAI's first-party endpoint
    /// is the only built-in that serves `/v1/responses`.
    #[test]
    fn responses_api_capability_is_coupled_to_the_preset_list() {
        for provider in BUILTIN_CLOUD_PROVIDERS {
            let expected = provider.slug == "openai";
            assert_eq!(
                builtin_cloud_supports_responses_api(provider.slug),
                expected,
                "built-in {} Responses-API capability drifted from the openai-only invariant",
                provider.slug
            );
        }
    }

    #[test]
    fn endpoint_host_parses_scheme_path_and_port() {
        assert_eq!(
            endpoint_host("https://integrate.api.nvidia.com/v1").as_deref(),
            Some("integrate.api.nvidia.com")
        );
        // Missing scheme, mixed case, trailing path.
        assert_eq!(
            endpoint_host("API.OpenAI.com/v1/chat").as_deref(),
            Some("api.openai.com")
        );
        // Userinfo + explicit port are stripped.
        assert_eq!(
            endpoint_host("https://user:pass@api.groq.com:443/openai/v1").as_deref(),
            Some("api.groq.com")
        );
        // Bracketed IPv6 literal with port.
        assert_eq!(
            endpoint_host("http://[::1]:8080/v1").as_deref(),
            Some("::1")
        );
        assert_eq!(endpoint_host("   ").as_deref(), None);
    }

    /// TAURI-RUST-HW1: the backend-URL resolver uses this to reroute backend
    /// domain calls away from a BYO inference host. Every built-in provider host
    /// must be recognised; OpenHuman backend hosts and unknown proxies must not.
    #[test]
    fn host_is_builtin_cloud_provider_recognises_inference_hosts() {
        for host in [
            "openrouter.ai",
            "api.openai.com",
            "api.anthropic.com",
            "api.groq.com",
            "generativelanguage.googleapis.com",
            "API.OPENAI.COM", // case-insensitive
        ] {
            assert!(
                host_is_builtin_cloud_provider(host),
                "{host} is a built-in cloud inference host"
            );
        }
        for host in [
            "api.tinyhumans.ai",
            "staging-api.tinyhumans.ai",
            "my-backend.example",
            "",
        ] {
            assert!(
                !host_is_builtin_cloud_provider(host),
                "{host:?} is not a built-in cloud inference host"
            );
        }
        // Every registry endpoint's own host must classify as builtin.
        for provider in BUILTIN_CLOUD_PROVIDERS {
            let host = endpoint_host(provider.endpoint).expect("preset endpoint has a host");
            assert!(
                host_is_builtin_cloud_provider(&host),
                "{} ({host}) must be recognised",
                provider.slug
            );
        }
    }

    /// TAURI-RUST-5A1: a *custom* slug pointed at a known chat-only host (NVIDIA)
    /// must be classified chat-only so the factory disables the guaranteed-404
    /// `/responses` fallback — the builtin-slug gate alone misses this because
    /// the slug is not builtin.
    #[test]
    fn nvidia_host_is_chat_completions_only_regardless_of_slug() {
        assert!(endpoint_host_is_chat_completions_only(
            "https://integrate.api.nvidia.com/v1"
        ));
        // Other chat-only built-in hosts too.
        for endpoint in [
            "https://api.deepseek.com/v1",
            "https://api.groq.com/openai/v1",
            "https://api.mistral.ai/v1",
        ] {
            assert!(
                endpoint_host_is_chat_completions_only(endpoint),
                "{endpoint} is a chat-completions-only built-in host"
            );
        }
    }

    #[test]
    fn openai_host_and_unknown_proxies_keep_the_responses_fallback() {
        // OpenAI's first-party host serves /responses — must NOT be gated off,
        // even via a custom proxy slug pointed at it.
        assert!(!endpoint_host_is_chat_completions_only(
            "https://api.openai.com/v1"
        ));
        // Genuinely unknown proxy hosts keep the permissive default (they may be
        // real OpenAI proxies that implement /responses).
        for endpoint in [
            "https://my-llm-proxy.internal.example/v1",
            "https://litellm.mycorp.dev/v1",
            "",
        ] {
            assert!(
                !endpoint_host_is_chat_completions_only(endpoint),
                "{endpoint:?} is an unknown host and must keep the fallback"
            );
        }
    }

    /// Drift guard: the host-based gate must agree with the slug-based
    /// capability for every built-in preset's own endpoint, so adding a preset
    /// can't silently desync the two gates.
    #[test]
    fn host_gate_agrees_with_slug_capability_for_every_builtin() {
        for provider in BUILTIN_CLOUD_PROVIDERS {
            // OpenhumanJwt / Anthropic presets never route through the
            // OpenAI-compatible Responses fallback; the gate only matters for
            // the Bearer OpenAI-compatible hosts.
            if provider.auth_style != AuthStyle::Bearer {
                continue;
            }
            let host_chat_only = endpoint_host_is_chat_completions_only(provider.endpoint);
            let slug_supports = builtin_cloud_supports_responses_api(provider.slug);
            assert_eq!(
                host_chat_only, !slug_supports,
                "host gate for built-in {} disagrees with its slug capability",
                provider.slug
            );
        }
    }
}
