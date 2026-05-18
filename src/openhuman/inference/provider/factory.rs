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
use crate::openhuman::inference::provider::compatible::{
    AuthStyle as CompatAuthStyle, OpenAiCompatibleProvider,
};
use crate::openhuman::inference::provider::openhuman_backend::OpenHumanBackendProvider;
use crate::openhuman::inference::provider::traits::Provider;
use crate::openhuman::inference::provider::ProviderRuntimeOptions;

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
        // When no explicit per-workload provider is set, resolve
        // primary_cloud.  If it points to a non-openhuman entry, route
        // there so users can use their own LLM provider without having
        // to set every single workload knob.  (An active app-session
        // is still required — verified inside
        // create_chat_provider_from_string.)
        let primary_slug = config.primary_cloud.as_deref().and_then(|pid| {
            config
                .cloud_providers
                .iter()
                .find(|e| e.id == pid && e.slug != "openhuman")
                .map(|e| e.slug.clone())
        });
        if let Some(slug) = primary_slug {
            format!("{slug}:")
        } else {
            PROVIDER_OPENHUMAN.to_string()
        }
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

    // ── Session gate ──────────────────────────────────────────────────
    // Custom providers (Ollama, <slug>:<model>) require an active
    // OpenHuman session.  Without this check an unregistered user can
    // point every workload at a custom provider and bypass the session
    // requirement entirely.
    //
    // Gate is skipped under #[cfg(test)] so existing unit tests that
    // create custom providers against a default Config continue to
    // pass.  The verify_session_active function itself is tested
    // explicitly with tempdir-backed auth profiles.
    #[cfg(not(test))]
    {
        verify_session_active(config)?;
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

/// Verify the user has an active OpenHuman backend session.
///
/// Without this check, an unregistered user can configure every workload
/// to use a custom cloud provider and bypass the session requirement
/// entirely.  This function ensures that custom providers (Ollama,
/// `<slug>:<model>`) are only reachable when the workspace holds a valid
/// `app-session` JWT.
fn verify_session_active(config: &Config) -> anyhow::Result<()> {
    // Fast path: the scheduler gate already knows the session is dead.
    if crate::openhuman::scheduler_gate::is_signed_out() {
        anyhow::bail!(
            "SESSION_EXPIRED: backend session not active — sign in to use custom providers"
        );
    }
    // Verify the app-session JWT actually exists in auth-profiles.
    let state_dir = config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|d| d.home_dir().join(".openhuman"))
                .unwrap_or_else(|| std::path::PathBuf::from(".openhuman"))
        });
    let auth = AuthService::new(&state_dir, config.secrets.encrypt);
    let has_session = auth
        .get_provider_bearer_token(crate::openhuman::credentials::APP_SESSION_PROVIDER, None)?
        .filter(|s| !s.trim().is_empty())
        .is_some();
    if !has_session {
        anyhow::bail!("SESSION_EXPIRED: no backend session — sign in to use OpenHuman")
    }
    Ok(())
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
    let p = make_openai_compatible_provider_with_config(
        &endpoint,
        "",
        CompatAuthStyle::None,
        &config.temperature_unsupported_models,
    )?;
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

    let unsupported = &config.temperature_unsupported_models;
    match entry.auth_style {
        AuthStyle::Anthropic => {
            let p = make_openai_compatible_provider_with_config(
                &entry.endpoint,
                &key,
                CompatAuthStyle::Anthropic,
                unsupported,
            )?;
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
            let p = make_openai_compatible_provider_with_config(
                &entry.endpoint,
                "",
                CompatAuthStyle::None,
                unsupported,
            )?;
            Ok((p, effective_model))
        }
        AuthStyle::Bearer => {
            let p = make_openai_compatible_provider_with_config(
                &entry.endpoint,
                &key,
                CompatAuthStyle::Bearer,
                unsupported,
            )?;
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
    make_openai_compatible_provider_with_config(endpoint, api_key, auth_style, &[])
}

/// Build an `OpenAiCompatibleProvider` with auth style and temperature
/// suppression list from config.
fn make_openai_compatible_provider_with_config(
    endpoint: &str,
    api_key: &str,
    auth_style: CompatAuthStyle,
    temperature_unsupported_models: &[String],
) -> anyhow::Result<Box<dyn Provider>> {
    let key = if api_key.trim().is_empty() {
        None
    } else {
        Some(api_key)
    };
    Ok(Box::new(
        OpenAiCompatibleProvider::new("cloud", endpoint, key, auth_style)
            .with_temperature_unsupported_models(temperature_unsupported_models.to_vec()),
    ))
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
#[path = "factory_test.rs"]
mod factory_test;
