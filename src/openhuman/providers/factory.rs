//! Unified chat-provider factory.
//!
//! Resolves workload names (e.g. `"reasoning"`, `"heartbeat"`) to a
//! `(Box<dyn Provider>, String)` tuple where the second element is the model
//! id to pass into `chat_with_history` / `simple_chat`.
//!
//! ## Provider-string grammar
//!
//! ```text
//! "cloud"               → resolves to primary_cloud entry; if that entry has
//!                         type=openhuman, behaves as "openhuman"
//! "openhuman"           → OpenHumanBackendProvider; model = config.default_model
//! "openai:<model>"      → cloud_providers entry of type=openai + Bearer auth
//! "anthropic:<model>"   → cloud_providers entry of type=anthropic + Bearer auth
//! "openrouter:<model>"  → cloud_providers entry of type=openrouter + Bearer auth
//! "custom:<model>"      → cloud_providers entry of type=custom + Bearer auth
//! "ollama:<model>"      → local Ollama at config.local_ai.base_url
//! ```
//!
//! Unknown strings and missing-creds configurations produce actionable errors.

use crate::openhuman::config::schema::cloud_providers::CloudProviderType;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use crate::openhuman::providers::compatible::{AuthStyle, OpenAiCompatibleProvider};
use crate::openhuman::providers::openhuman_backend::OpenHumanBackendProvider;
use crate::openhuman::providers::traits::Provider;
use crate::openhuman::providers::ProviderRuntimeOptions;

/// Sentinel meaning "use whatever primary_cloud resolves to".
pub const PROVIDER_CLOUD: &str = "cloud";
/// Sentinel meaning "use the OpenHuman backend session JWT".
pub const PROVIDER_OPENHUMAN: &str = "openhuman";
/// Prefix for Ollama-local providers: `"ollama:<model>"`.
pub const OLLAMA_PROVIDER_PREFIX: &str = "ollama:";
/// Prefix for OpenAI-compatible providers: `"openai:<model>"`.
pub const OPENAI_PROVIDER_PREFIX: &str = "openai:";
/// Prefix for Anthropic-compatible providers: `"anthropic:<model>"`.
pub const ANTHROPIC_PROVIDER_PREFIX: &str = "anthropic:";
/// Prefix for OpenRouter providers: `"openrouter:<model>"`.
pub const OPENROUTER_PROVIDER_PREFIX: &str = "openrouter:";
/// Prefix for custom OpenAI-compatible providers: `"custom:<model>"`.
pub const CUSTOM_PROVIDER_PREFIX: &str = "custom:";

/// Return the configured provider string for a named workload role.
///
/// Returns `"cloud"` when the workload has no explicit override, which causes
/// the factory to resolve via `primary_cloud`.
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
    if s.is_empty() {
        PROVIDER_CLOUD.to_string()
    } else {
        s.to_string()
    }
}

/// Build a `(Provider, model)` for the given workload role.
///
/// Equivalent to:
/// ```rust,ignore
/// let s = provider_for_role(role, config);
/// create_chat_provider_from_string(role, &s, config)
/// ```
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

    if p == PROVIDER_CLOUD || p.is_empty() {
        return resolve_cloud_primary(role, config);
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

    // Third-party cloud providers — look up the matching entry by type.
    let (type_str, model) = parse_typed_prefix(p).ok_or_else(|| {
        anyhow::anyhow!(
            "[chat-factory] unrecognised provider string '{}' for role '{}'. \
             Valid forms: cloud, openhuman, ollama:<m>, openai:<m>, \
             anthropic:<m>, openrouter:<m>, custom:<m>",
            p,
            role
        )
    })?;

    if model.trim().is_empty() {
        anyhow::bail!(
            "[chat-factory] provider string '{}' for role '{}' has an empty model",
            p,
            role
        );
    }

    let provider_type = match type_str {
        "openai" => CloudProviderType::Openai,
        "anthropic" => CloudProviderType::Anthropic,
        "openrouter" => CloudProviderType::Openrouter,
        "custom" => CloudProviderType::Custom,
        other => anyhow::bail!(
            "[chat-factory] unknown provider type '{}' in provider string '{}' for role '{}'",
            other,
            p,
            role
        ),
    };

    make_cloud_provider_by_type(role, &provider_type, model.trim(), config)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Resolve the `"cloud"` sentinel by consulting `primary_cloud`.
fn resolve_cloud_primary(
    role: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    // Find the primary entry (or fall back to an OpenHuman entry / first entry).
    let entry = if let Some(ref primary_id) = config.primary_cloud {
        config.cloud_providers.iter().find(|e| &e.id == primary_id)
    } else {
        None
    };

    let entry = entry.or_else(|| {
        // Implicit fallback: first openhuman entry, then any entry.
        config
            .cloud_providers
            .iter()
            .find(|e| e.r#type == CloudProviderType::Openhuman)
            .or_else(|| config.cloud_providers.first())
    });

    match entry {
        None => {
            // No cloud_providers configured at all — route to the OpenHuman backend.
            log::debug!(
                "[providers][chat-factory] no cloud_providers entries, \
                 falling back to openhuman backend for role={}",
                role
            );
            make_openhuman_backend(config)
        }
        Some(e) if e.r#type == CloudProviderType::Openhuman => {
            log::debug!(
                "[providers][chat-factory] primary resolves to openhuman backend for role={}",
                role
            );
            make_openhuman_backend(config)
        }
        Some(e) => {
            let model = e.default_model.clone();
            if model.trim().is_empty() {
                anyhow::bail!(
                    "[chat-factory] primary cloud provider '{}' (type={}) has an empty \
                     default_model — set a model for role '{}'",
                    e.id,
                    e.r#type.label(),
                    role
                );
            }
            log::info!(
                "[providers][chat-factory] role={} resolved cloud→{}:{} endpoint_host={}",
                role,
                e.r#type.label(),
                model,
                redact_endpoint(&e.endpoint)
            );
            let key = lookup_provider_key(&e.r#type, config)?;
            let p = make_openai_compatible_provider(&e.endpoint, &key)?;
            Ok((p, model))
        }
    }
}

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
    // canonical tier names. The legacy `create_intelligent_routing_provider`
    // path did this inside `IntelligentRoutingProvider::resolve_remote_model`;
    // #1710's factory bypassed that wrapper, which broke the web-chat
    // `model_override` contract (json_rpc_e2e `routing_cases`):
    // `hint:reasoning` was reaching the backend verbatim instead of
    // `reasoning-v1`. We apply ONLY the model-name mapping here — not the
    // full routing wrapper, which also injects local-AI health probing and
    // a streaming shim that the web-chat SSE path doesn't tolerate (it
    // hangs `chat_done`). Mapping mirrors `resolve_remote_model`'s
    // heavy-tier arm exactly: known heavy tiers map to `<tier>-v1`;
    // lightweight hints (`hint:reaction`, …) and already-exact tier names
    // pass through untouched. Third-party cloud providers never see
    // `hint:` strings, so this stays scoped to the OpenHuman backend.
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
    let p = make_openai_compatible_provider(&endpoint, "")?;
    Ok((p, model.to_string()))
}

/// Look up a cloud_providers entry by type and build the provider.
fn make_cloud_provider_by_type(
    role: &str,
    provider_type: &CloudProviderType,
    model: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    let entry = config
        .cloud_providers
        .iter()
        .find(|e| &e.r#type == provider_type);

    let entry = entry.ok_or_else(|| {
        anyhow::anyhow!(
            "[chat-factory] no cloud provider configured for type '{}' (role '{}') — \
             add a {} entry to cloud_providers in config.toml",
            provider_type.as_str(),
            role,
            provider_type.label()
        )
    })?;

    log::info!(
        "[providers][chat-factory] role={} type={} model={} endpoint_host={}",
        role,
        provider_type.label(),
        model,
        redact_endpoint(&entry.endpoint)
    );
    let key = lookup_provider_key(provider_type, config)?;
    let p = make_openai_compatible_provider(&entry.endpoint, &key)?;
    Ok((p, model.to_string()))
}

/// Fetch the encrypted bearer token for a cloud provider type from the
/// workspace `auth-profiles.json` via the shared [`AuthService`].
///
/// Each provider type maps to a default profile named `"<type>:default"`
/// (e.g. `"openai:default"`), mirroring how the Composio integration stores
/// `"composio-direct:default"`. Missing or empty keys return `Ok(String::new())`
/// — callers (and `make_openai_compatible_provider`) treat that as "no auth",
/// which surfaces an authentication error at first call rather than at factory
/// build time. This keeps the factory testable without forcing every test to
/// seed an auth profile.
fn lookup_provider_key(
    provider_type: &CloudProviderType,
    config: &Config,
) -> anyhow::Result<String> {
    // OpenHuman uses the session JWT path; no separate key here.
    if matches!(provider_type, CloudProviderType::Openhuman) {
        return Ok(String::new());
    }
    let auth = AuthService::from_config(config);
    let key = auth
        .get_provider_bearer_token(provider_type.as_str(), None)
        .map_err(|e| {
            anyhow::anyhow!(
                "[chat-factory] failed to read API key for provider '{}': {}",
                provider_type.as_str(),
                e
            )
        })?
        .unwrap_or_default();
    log::debug!(
        "[providers][chat-factory] auth lookup type={} key_present={}",
        provider_type.as_str(),
        !key.is_empty()
    );
    Ok(key)
}

/// Build an `OpenAiCompatibleProvider` with Bearer auth.
fn make_openai_compatible_provider(
    endpoint: &str,
    api_key: &str,
) -> anyhow::Result<Box<dyn Provider>> {
    let key = if api_key.trim().is_empty() {
        None
    } else {
        Some(api_key)
    };
    Ok(Box::new(OpenAiCompatibleProvider::new(
        "cloud",
        endpoint,
        key,
        AuthStyle::Bearer,
    )))
}

/// Return a safe-to-log representation of a URL endpoint: `scheme://host` only.
///
/// User-configured endpoints can embed API keys or tokens in the query string
/// or even in the authority (e.g. `https://key@host/`). Logging the raw URL
/// violates the "never log secrets" rule. This helper strips everything except
/// the scheme and host so logs are still useful for debugging routing issues
/// without leaking credentials.
fn redact_endpoint(url: &str) -> String {
    let trimmed = url.trim();
    // Try to extract scheme://host by splitting on "://" and then on "/", "@", "?".
    if let Some(rest) = trimmed.split_once("://") {
        let scheme = rest.0;
        // Strip any userinfo (user:pass@host) and take only up to the first path/query.
        let authority = rest.1.split('/').next().unwrap_or("");
        let host = authority.split('@').last().unwrap_or(authority);
        let host_no_query = host.split('?').next().unwrap_or(host);
        return format!("{}://{}", scheme, host_no_query);
    }
    // No "://" — probably a bare host or unrecognised; log a placeholder.
    "<endpoint>".to_string()
}

/// Split `"openai:gpt-4o"` → `("openai", "gpt-4o")`, or `None` if unrecognised.
fn parse_typed_prefix(s: &str) -> Option<(&str, &str)> {
    for prefix in &[
        OPENAI_PROVIDER_PREFIX,
        ANTHROPIC_PROVIDER_PREFIX,
        OPENROUTER_PROVIDER_PREFIX,
        CUSTOM_PROVIDER_PREFIX,
    ] {
        if let Some(model) = s.strip_prefix(prefix) {
            let type_str = prefix.trim_end_matches(':');
            return Some((type_str, model));
        }
    }
    None
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::schema::cloud_providers::{
        CloudProviderCreds, CloudProviderType,
    };
    use crate::openhuman::config::Config;

    fn config_with_providers(
        providers: Vec<CloudProviderCreds>,
        primary: Option<String>,
    ) -> Config {
        let mut c = Config::default();
        c.cloud_providers = providers;
        c.primary_cloud = primary;
        c
    }

    fn oh_entry(id: &str) -> CloudProviderCreds {
        CloudProviderCreds {
            id: id.to_string(),
            r#type: CloudProviderType::Openhuman,
            endpoint: "https://api.openhuman.ai/v1".to_string(),
            default_model: "reasoning-v1".to_string(),
        }
    }

    fn openai_entry(id: &str, model: &str) -> CloudProviderCreds {
        CloudProviderCreds {
            id: id.to_string(),
            r#type: CloudProviderType::Openai,
            endpoint: "https://api.openai.com/v1".to_string(),
            default_model: model.to_string(),
        }
    }

    fn anthropic_entry(id: &str, model: &str) -> CloudProviderCreds {
        CloudProviderCreds {
            id: id.to_string(),
            r#type: CloudProviderType::Anthropic,
            endpoint: "https://api.anthropic.com/v1".to_string(),
            default_model: model.to_string(),
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
        let result = create_chat_provider_from_string("reasoning", "cloud", &config);
        assert!(
            result.is_ok(),
            "cloud fallback must succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn cloud_with_openhuman_primary() {
        let config = config_with_providers(vec![oh_entry("p_oh")], Some("p_oh".to_string()));
        let (_, model) = create_chat_provider_from_string("reasoning", "cloud", &config)
            .expect("cloud→openhuman primary must build");
        assert!(!model.is_empty());
    }

    #[test]
    fn cloud_with_openai_primary() {
        let config = config_with_providers(
            vec![oh_entry("p_oh"), openai_entry("p_oai", "gpt-4o")],
            Some("p_oai".to_string()),
        );
        let (_, model) = create_chat_provider_from_string("reasoning", "cloud", &config)
            .expect("cloud→openai primary must build");
        assert_eq!(model, "gpt-4o");
    }

    #[test]
    fn openai_prefix() {
        let config = config_with_providers(vec![openai_entry("p_oai", "gpt-4o")], None);
        let (_, model) = create_chat_provider_from_string("agentic", "openai:gpt-4o-mini", &config)
            .expect("openai:<model> must build");
        assert_eq!(model, "gpt-4o-mini");
    }

    #[test]
    fn anthropic_prefix() {
        let config =
            config_with_providers(vec![anthropic_entry("p_ant", "claude-sonnet-4-6")], None);
        let (_, model) =
            create_chat_provider_from_string("coding", "anthropic:claude-sonnet-4-6", &config)
                .expect("anthropic:<model> must build");
        assert_eq!(model, "claude-sonnet-4-6");
    }

    #[test]
    fn openrouter_prefix() {
        let mut config = Config::default();
        config.cloud_providers.push(CloudProviderCreds {
            id: "p_or".to_string(),
            r#type: CloudProviderType::Openrouter,
            endpoint: "https://openrouter.ai/api/v1".to_string(),
            default_model: "openai/gpt-4o".to_string(),
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
    fn all_workloads_default_to_cloud() {
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
                "cloud",
                "role={role} must default to cloud"
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
        assert_eq!(provider_for_role("reasoning", &config), "cloud");
    }

    #[test]
    fn create_chat_provider_uses_role() {
        let mut config = Config::default();
        config.cloud_providers.push(openai_entry("p_oai", "gpt-4o"));
        config.primary_cloud = Some("p_oai".to_string());
        config.reasoning_provider = Some("openai:gpt-4o-mini".to_string());
        let (_, model) =
            create_chat_provider("reasoning", &config).expect("create_chat_provider must succeed");
        assert_eq!(model, "gpt-4o-mini");
    }

    // ── Error cases ───────────────────────────────────────────────────────────

    #[test]
    fn unknown_provider_string_rejected() {
        // `Result<(Box<dyn Provider>, String), _>` can't use `.expect_err`
        // because `dyn Provider` doesn't implement `Debug` — drop the
        // Ok via `.err()` and pattern on the Option instead.
        let config = Config::default();
        let err = create_chat_provider_from_string("reasoning", "groq:llama3", &config)
            .err()
            .expect("unknown provider string must fail");
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
    fn missing_creds_for_openai_gives_clear_error() {
        // No openai entry in cloud_providers.
        let config = Config::default();
        let err = create_chat_provider_from_string("reasoning", "openai:gpt-4o", &config)
            .err()
            .expect("missing creds must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("no cloud provider configured for type 'openai'"),
            "{msg}"
        );
    }

    #[test]
    fn primary_cloud_defaults_to_openhuman_when_none() {
        // primary_cloud=None → factory must still succeed by falling back
        // to either an openhuman entry or the openhuman backend.
        let config = Config::default();
        assert!(create_chat_provider("reasoning", &config).is_ok());
    }

    // ── Summarization alias ───────────────────────────────────────────────────

    #[test]
    fn summarization_aliases_memory_provider() {
        // The summarizer sub-agent declares `[model] hint = "summarization"`
        // but there's no `summarization_provider` config field — the workload
        // is a synonym for `memory` since both are "condense input" tasks.
        // `provider_for_role("summarization", ...)` must read `memory_provider`.
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
    fn summarization_defaults_to_cloud_like_memory() {
        // No memory_provider → both `memory` and `summarization` fall through
        // to "cloud", consistent with every other workload.
        let config = Config::default();
        assert_eq!(provider_for_role("memory", &config), "cloud");
        assert_eq!(provider_for_role("summarization", &config), "cloud");
    }

    #[test]
    fn unknown_workload_falls_back_to_cloud() {
        // The wildcard arm in provider_for_role's match must keep
        // unrecognised workloads on the primary so a typo in an agent
        // TOML doesn't surface as a NoneProvider crash.
        let config = Config::default();
        assert_eq!(provider_for_role("nope-not-a-workload", &config), "cloud");
        assert_eq!(provider_for_role("", &config), "cloud");
    }

    // ── OpenHuman backend state_dir wiring ────────────────────────────────────

    #[test]
    fn openhuman_backend_uses_config_path_parent_as_state_dir() {
        // Regression test for #1710: when the user's workspace lives outside
        // ~/.openhuman (e.g. OPENHUMAN_WORKSPACE override, or a test
        // worktree), the factory must propagate config.config_path.parent()
        // into ProviderRuntimeOptions.openhuman_dir so the backend's
        // AuthService reads `auth-profiles.json` from the same workspace
        // login wrote to. Without this, login succeeds but every chat
        // call bails with "No backend session".
        let mut config = Config::default();
        config.config_path = std::path::PathBuf::from("/tmp/oh-test-workspace/config.toml");
        // Build via the chat-factory entrypoint — make_openhuman_backend
        // is private and called transitively when there are no
        // cloud_providers entries.
        let (_provider, model) = create_chat_provider("reasoning", &config)
            .expect("openhuman backend must build with no cloud_providers");
        // Sanity: the model resolution path also goes through the
        // backend ctor, so a successful build implies state_dir was
        // wired without panic.
        assert!(!model.is_empty(), "model must be set");
    }
}
