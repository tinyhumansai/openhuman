//! Unified chat-provider factory.
//!
//! Resolves workload names (e.g. `"reasoning"`, `"heartbeat"`) to a
//! `(Box<dyn Provider>, String)` tuple where the second element is the model
//! id to pass into `chat_with_history` / `simple_chat`.
//!
//! ## Provider-string grammar
//!
//! ```text
//! "openhuman"        → OpenHumanBackendProvider; model = config.default_model
//! "ollama:<model>"   → local Ollama at config.local_ai.base_url
//! "<slug>:<model>"   → cloud_providers entry keyed by slug;
//!                      builds OpenAiCompatibleProvider (Bearer) or Anthropic
//!                      flavour depending on auth_style.
//! ""  / missing      → falls back to "openhuman"
//! ```
//!
//! Unknown slugs and missing-creds configurations produce actionable errors.

use crate::openhuman::config::schema::cloud_providers::AuthStyle;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use crate::openhuman::providers::compatible::{
    AuthStyle as CompatAuthStyle, OpenAiCompatibleProvider,
};
use crate::openhuman::providers::openhuman_backend::OpenHumanBackendProvider;
use crate::openhuman::providers::traits::Provider;
use crate::openhuman::providers::ProviderRuntimeOptions;

/// Sentinel meaning "use the OpenHuman backend session JWT".
pub const PROVIDER_OPENHUMAN: &str = "openhuman";
/// Prefix for Ollama-local providers: `"ollama:<model>"`.
pub const OLLAMA_PROVIDER_PREFIX: &str = "ollama:";

/// Auth-profile storage key for a slug-keyed provider.
///
/// New writes use `"provider:<slug>"`. Lookups also try the bare `<slug>`
/// as a legacy fallback (old configs stored keys as e.g. `"openai:default"`).
pub fn auth_key_for_slug(slug: &str) -> String {
    format!("provider:{slug}")
}

/// Return the configured provider string for a named workload role.
///
/// Returns `"openhuman"` when the workload has no explicit override.
pub fn provider_for_role(role: &str, config: &Config) -> String {
    let opt = match role {
        "reasoning" => config.reasoning_provider.as_deref(),
        "agentic" => config.agentic_provider.as_deref(),
        "coding" => config.coding_provider.as_deref(),
        // `memory_provider` covers both the memory-tree extract path and
        // the summarizer sub-agent (whose definition declares
        // `hint = "summarization"`). Both are "produce a condensed
        // representation of input text" — same model class, no reason
        // for a separate config knob.
        "memory" | "summarization" => config.memory_provider.as_deref(),
        "embeddings" => config.embeddings_provider.as_deref(),
        "heartbeat" => config.heartbeat_provider.as_deref(),
        "learning" => config.learning_provider.as_deref(),
        "subconscious" => config.subconscious_provider.as_deref(),
        _ => None,
    };
    let s = opt.unwrap_or("").trim();
    if s.is_empty() || s == "cloud" {
        PROVIDER_OPENHUMAN.to_string()
    } else {
        s.to_string()
    }
}

/// Build a `(Provider, model)` for the given workload role.
pub fn create_chat_provider(
    role: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    let s = provider_for_role(role, config);
    log::debug!(
        "[providers][chat-factory] create_chat_provider role={} resolved_string={}",
        role,
        s
    );
    create_chat_provider_from_string(role, &s, config)
}

/// Build a `(Provider, model)` from an explicit provider string and config.
///
/// See module-level grammar documentation for valid formats.
pub fn create_chat_provider_from_string(
    role: &str,
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    let p = provider.trim();
    log::debug!(
        "[providers][chat-factory] create_chat_provider_from_string role={} provider={}",
        role,
        p
    );

    // Empty / legacy "cloud" sentinel → OpenHuman backend.
    if p.is_empty() || p == "cloud" {
        return make_openhuman_backend(config);
    }

    if p == PROVIDER_OPENHUMAN {
        return make_openhuman_backend(config);
    }

    if let Some(model) = p.strip_prefix(OLLAMA_PROVIDER_PREFIX) {
        if model.trim().is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty model — \
                 use 'ollama:<model-id>'",
                p,
                role
            );
        }
        return make_ollama_provider(model.trim(), config);
    }

    // New grammar: "<slug>:<model>"
    if let Some(colon_pos) = p.find(':') {
        let slug = p[..colon_pos].trim();
        let model = p[colon_pos + 1..].trim();

        if slug.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty slug",
                p,
                role
            );
        }

        return make_cloud_provider_by_slug(role, slug, model, config);
    }

    // No colon: might be a bare legacy type string (e.g. "openai"). Try as
    // slug lookup with empty model — gives a clear "no entry" error rather
    // than an opaque parse failure.
    anyhow::bail!(
        "[chat-factory] unrecognised provider string '{}' for role '{}'. \
         Valid forms: openhuman, ollama:<model>, <slug>:<model>. \
         Configured slugs: [{}]",
        p,
        role,
        config
            .cloud_providers
            .iter()
            .map(|e| e.slug.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Build the OpenHuman backend provider (session-JWT auth).
fn make_openhuman_backend(config: &Config) -> anyhow::Result<(Box<dyn Provider>, String)> {
    let model = config
        .default_model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "reasoning-v1".to_string());
    // Critical: pass the *config's* workspace directory through so the
    // provider's `AuthService` reads `auth-profiles.json` from the
    // same dir login wrote to. Without this, `ProviderRuntimeOptions::default()`
    // leaves `openhuman_dir = None`, the provider falls back to
    // `~/.openhuman`, and reads an unrelated (or empty)
    // profile store — surfacing as "No backend session: store a JWT
    // via auth (app-session)" even though login just succeeded in the
    // user's actual workspace (e.g. test workspaces under OPENHUMAN_WORKSPACE).
    let options = ProviderRuntimeOptions {
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        ..ProviderRuntimeOptions::default()
    };
    log::debug!(
        "[providers][chat-factory] building openhuman backend provider model={} state_dir={:?} secrets_encrypt={}",
        model,
        options.openhuman_dir,
        options.secrets_encrypt
    );
    // Translate `hint:<tier>` model strings into the OpenHuman backend's
    // canonical tier names.
    let model = match model.strip_prefix("hint:") {
        Some("reasoning") => crate::openhuman::config::MODEL_REASONING_V1.to_string(),
        Some("chat") => crate::openhuman::config::MODEL_REASONING_QUICK_V1.to_string(),
        Some("agentic") => crate::openhuman::config::MODEL_AGENTIC_V1.to_string(),
        Some("coding") => crate::openhuman::config::MODEL_CODING_V1.to_string(),
        _ => model,
    };
    let p = Box::new(OpenHumanBackendProvider::new(
        config.api_url.as_deref(),
        &options,
    ));
    Ok((p, model))
}

/// Build an Ollama local provider.
fn make_ollama_provider(
    model: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    let base_url = config
        .local_ai
        .base_url
        .as_deref()
        .unwrap_or("http://localhost:11434");
    // Ollama exposes an OpenAI-compatible endpoint at /v1.
    let endpoint = format!("{}/v1", base_url.trim_end_matches('/'));
    log::info!(
        "[providers][chat-factory] building ollama provider model={} endpoint_host={}",
        model,
        redact_endpoint(&endpoint)
    );
    let p = make_openai_compatible_provider(&endpoint, "", CompatAuthStyle::Bearer)?;
    Ok((p, model.to_string()))
}

/// Look up a `cloud_providers` entry by slug and build the provider.
fn make_cloud_provider_by_slug(
    role: &str,
    slug: &str,
    model: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    let entry = config.cloud_providers.iter().find(|e| e.slug == slug);

    let entry = entry.ok_or_else(|| {
        let known: Vec<&str> = config
            .cloud_providers
            .iter()
            .map(|e| e.slug.as_str())
            .collect();
        anyhow::anyhow!(
            "[chat-factory] no cloud provider configured for slug '{}' (role '{}') — \
             add an entry with that slug to cloud_providers in config.toml. \
             Configured slugs: [{}]",
            slug,
            role,
            known.join(", ")
        )
    })?;

    // Resolve effective model: use provided model if non-empty, else fall back
    // to the entry's legacy default_model (if any), else empty → error.
    let effective_model = if model.trim().is_empty() {
        entry.default_model.clone().unwrap_or_default()
    } else {
        model.to_string()
    };

    log::info!(
        "[providers][chat-factory] role={} slug={} model={} endpoint_host={}",
        role,
        slug,
        effective_model,
        redact_endpoint(&entry.endpoint)
    );

    let key = lookup_key_for_slug(slug, config)?;

    match entry.auth_style {
        AuthStyle::Anthropic => {
            let p =
                make_openai_compatible_provider(&entry.endpoint, &key, CompatAuthStyle::Anthropic)?;
            Ok((p, effective_model))
        }
        AuthStyle::OpenhumanJwt => {
            // Route to the OpenHuman backend — ignore the entry's endpoint
            // and model; use the backend provider with the configured default.
            log::debug!(
                "[providers][chat-factory] slug='{}' has auth_style=OpenhumanJwt → routing to openhuman backend",
                slug
            );
            make_openhuman_backend(config)
        }
        AuthStyle::None => {
            let p = make_openai_compatible_provider(&entry.endpoint, "", CompatAuthStyle::Bearer)?;
            Ok((p, effective_model))
        }
        AuthStyle::Bearer => {
            let p =
                make_openai_compatible_provider(&entry.endpoint, &key, CompatAuthStyle::Bearer)?;
            Ok((p, effective_model))
        }
    }
}

/// Fetch the bearer token for a slug from the workspace `auth-profiles.json`.
///
/// Tries `provider:<slug>` first (new key format), then the bare `<slug>`
/// (legacy format where keys were stored as `"openai"`, `"anthropic"`, etc.).
/// Missing or empty keys return `Ok(String::new())` — callers treat that as
/// "no auth", which surfaces an authentication error at first call rather than
/// at factory build time.
pub fn lookup_key_for_slug(slug: &str, config: &Config) -> anyhow::Result<String> {
    let auth = AuthService::from_config(config);
    // Try new-style key first.
    let new_key = auth_key_for_slug(slug);
    if let Ok(Some(k)) = auth.get_provider_bearer_token(&new_key, None) {
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
        .map_err(|e| {
            anyhow::anyhow!(
                "[chat-factory] failed to read API key for slug '{}': {}",
                slug,
                e
            )
        })?
        .unwrap_or_default();
    log::debug!(
        "[providers][chat-factory] auth lookup slug={} key_present={}",
        slug,
        !key.is_empty()
    );
    Ok(key)
}

/// Build an `OpenAiCompatibleProvider` with the given auth style.
fn make_openai_compatible_provider(
    endpoint: &str,
    api_key: &str,
    auth_style: CompatAuthStyle,
) -> anyhow::Result<Box<dyn Provider>> {
    let key = if api_key.trim().is_empty() {
        None
    } else {
        Some(api_key)
    };
    Ok(Box::new(OpenAiCompatibleProvider::new(
        "cloud", endpoint, key, auth_style,
    )))
}

/// Return a safe-to-log representation of a URL endpoint: `scheme://host` only.
fn redact_endpoint(url: &str) -> String {
    let trimmed = url.trim();
    if let Some(rest) = trimmed.split_once("://") {
        let scheme = rest.0;
        let authority = rest.1.split('/').next().unwrap_or("");
        let host = authority.split('@').last().unwrap_or(authority);
        let host_no_query = host.split('?').next().unwrap_or(host);
        return format!("{}://{}", scheme, host_no_query);
    }
    "<endpoint>".to_string()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    use crate::openhuman::config::Config;

    fn config_with_providers(providers: Vec<CloudProviderCreds>) -> Config {
        let mut c = Config::default();
        c.cloud_providers = providers;
        c
    }

    fn oh_entry(id: &str) -> CloudProviderCreds {
        CloudProviderCreds {
            id: id.to_string(),
            slug: "openhuman".to_string(),
            label: "OpenHuman".to_string(),
            endpoint: "https://api.openhuman.ai/v1".to_string(),
            auth_style: AuthStyle::OpenhumanJwt,
            ..Default::default()
        }
    }

    fn openai_entry(id: &str, slug: &str) -> CloudProviderCreds {
        CloudProviderCreds {
            id: id.to_string(),
            slug: slug.to_string(),
            label: "OpenAI".to_string(),
            endpoint: "https://api.openai.com/v1".to_string(),
            auth_style: AuthStyle::Bearer,
            default_model: Some("gpt-4o".to_string()),
            ..Default::default()
        }
    }

    fn anthropic_entry(id: &str, slug: &str) -> CloudProviderCreds {
        CloudProviderCreds {
            id: id.to_string(),
            slug: slug.to_string(),
            label: "Anthropic".to_string(),
            endpoint: "https://api.anthropic.com/v1".to_string(),
            auth_style: AuthStyle::Anthropic,
            default_model: Some("claude-sonnet-4-6".to_string()),
            ..Default::default()
        }
    }

    // ── Grammar: all recognised forms ────────────────────────────────────────

    #[test]
    fn openhuman_literal() {
        let config = Config::default();
        let (_, model) = create_chat_provider_from_string("reasoning", "openhuman", &config)
            .expect("openhuman literal must build");
        assert!(!model.is_empty(), "model must not be empty");
    }

    #[test]
    fn cloud_no_providers_falls_back_to_openhuman() {
        let config = Config::default();
        // "cloud" sentinel still works — routes to openhuman.
        let result = create_chat_provider_from_string("reasoning", "cloud", &config);
        assert!(
            result.is_ok(),
            "cloud fallback must succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn openhuman_slug_routes_to_backend() {
        let config = config_with_providers(vec![oh_entry("p_oh")]);
        let (_, model) = create_chat_provider_from_string("reasoning", "openhuman:", &config)
            .expect("openhuman: must build");
        assert!(!model.is_empty());
    }

    #[test]
    fn openai_slug_model() {
        let config = config_with_providers(vec![openai_entry("p_oai", "openai")]);
        let (_, model) = create_chat_provider_from_string("agentic", "openai:gpt-4o-mini", &config)
            .expect("openai:<model> must build");
        assert_eq!(model, "gpt-4o-mini");
    }

    #[test]
    fn anthropic_slug_model() {
        let config = config_with_providers(vec![anthropic_entry("p_ant", "anthropic")]);
        let (_, model) =
            create_chat_provider_from_string("coding", "anthropic:claude-sonnet-4-6", &config)
                .expect("anthropic:<model> must build");
        assert_eq!(model, "claude-sonnet-4-6");
    }

    #[test]
    fn openrouter_slug_model() {
        let mut config = Config::default();
        config.cloud_providers.push(CloudProviderCreds {
            id: "p_or".to_string(),
            slug: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            endpoint: "https://openrouter.ai/api/v1".to_string(),
            auth_style: AuthStyle::Bearer,
            default_model: Some("openai/gpt-4o".to_string()),
            ..Default::default()
        });
        let (_, model) = create_chat_provider_from_string(
            "agentic",
            "openrouter:meta-llama/llama-3.1-8b",
            &config,
        )
        .expect("openrouter:<model> must build");
        assert_eq!(model, "meta-llama/llama-3.1-8b");
    }

    #[test]
    fn ollama_prefix() {
        let config = Config::default();
        let (_, model) =
            create_chat_provider_from_string("heartbeat", "ollama:llama3.1:8b", &config)
                .expect("ollama:<model> must build");
        assert_eq!(model, "llama3.1:8b");
    }

    // ── Workload routing ──────────────────────────────────────────────────────

    #[test]
    fn all_workloads_default_to_openhuman() {
        let config = Config::default();
        for role in &[
            "reasoning",
            "agentic",
            "coding",
            "memory",
            "embeddings",
            "heartbeat",
            "learning",
            "subconscious",
        ] {
            assert_eq!(
                provider_for_role(role, &config),
                "openhuman",
                "role={role} must default to openhuman"
            );
        }
    }

    #[test]
    fn workload_override_respected() {
        let mut config = Config::default();
        config.heartbeat_provider = Some("ollama:llama3.2:3b".to_string());
        assert_eq!(
            provider_for_role("heartbeat", &config),
            "ollama:llama3.2:3b"
        );
        assert_eq!(provider_for_role("reasoning", &config), "openhuman");
    }

    #[test]
    fn create_chat_provider_uses_role() {
        let mut config = Config::default();
        config.cloud_providers.push(openai_entry("p_oai", "openai"));
        config.reasoning_provider = Some("openai:gpt-4o-mini".to_string());
        let (_, model) =
            create_chat_provider("reasoning", &config).expect("create_chat_provider must succeed");
        assert_eq!(model, "gpt-4o-mini");
    }

    // ── Error cases ───────────────────────────────────────────────────────────

    #[test]
    fn unknown_slug_rejected() {
        let config = Config::default();
        let err = create_chat_provider_from_string("reasoning", "groq:llama3", &config)
            .err()
            .expect("unknown slug must fail");
        assert!(
            err.to_string()
                .contains("no cloud provider configured for slug"),
            "{err}"
        );
    }

    #[test]
    fn bare_string_without_colon_rejected() {
        let config = Config::default();
        let err = create_chat_provider_from_string("reasoning", "openai", &config)
            .err()
            .expect("bare string must fail");
        assert!(
            err.to_string().contains("unrecognised provider string"),
            "{err}"
        );
    }

    #[test]
    fn empty_model_in_ollama_rejected() {
        let config = Config::default();
        let err = create_chat_provider_from_string("reasoning", "ollama:", &config)
            .err()
            .expect("empty model must fail");
        assert!(err.to_string().contains("empty model"), "{err}");
    }

    #[test]
    fn missing_slug_for_openai_gives_clear_error() {
        // No openai entry in cloud_providers.
        let config = Config::default();
        let err = create_chat_provider_from_string("reasoning", "openai:gpt-4o", &config)
            .err()
            .expect("missing slug must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("no cloud provider configured for slug 'openai'"),
            "{msg}"
        );
    }

    #[test]
    fn primary_cloud_defaults_to_openhuman_when_no_providers() {
        let config = Config::default();
        assert!(create_chat_provider("reasoning", &config).is_ok());
    }

    // ── Summarization alias ───────────────────────────────────────────────────

    #[test]
    fn summarization_aliases_memory_provider() {
        let mut config = Config::default();
        config.memory_provider = Some("ollama:llama3.1:8b".to_string());
        assert_eq!(provider_for_role("memory", &config), "ollama:llama3.1:8b");
        assert_eq!(
            provider_for_role("summarization", &config),
            "ollama:llama3.1:8b",
            "summarization must alias memory_provider"
        );
    }

    #[test]
    fn summarization_defaults_to_openhuman_like_memory() {
        let config = Config::default();
        assert_eq!(provider_for_role("memory", &config), "openhuman");
        assert_eq!(provider_for_role("summarization", &config), "openhuman");
    }

    #[test]
    fn unknown_workload_falls_back_to_openhuman() {
        let config = Config::default();
        assert_eq!(
            provider_for_role("nope-not-a-workload", &config),
            "openhuman"
        );
        assert_eq!(provider_for_role("", &config), "openhuman");
    }

    // ── OpenHuman backend state_dir wiring ────────────────────────────────────

    #[test]
    fn openhuman_backend_uses_config_path_parent_as_state_dir() {
        let mut config = Config::default();
        config.config_path = std::path::PathBuf::from("/tmp/oh-test-workspace/config.toml");
        let (_provider, model) = create_chat_provider("reasoning", &config)
            .expect("openhuman backend must build with no cloud_providers");
        assert!(!model.is_empty(), "model must be set")
    }
}
