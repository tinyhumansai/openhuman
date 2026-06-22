//! Unified chat-provider factory.
//!
//! Resolves workload names (e.g. `"reasoning"`, `"heartbeat"`) to a
//! `(Box<dyn Provider>, String)` tuple where the second element is the model
//! id to pass into `chat_with_history` / `simple_chat`.
//!
//! ## Provider-string grammar
//!
//! ```text
//! "openhuman"                    → OpenHumanBackendProvider; model = config.default_model
//! "cloud" / missing              → primary_cloud; legacy custom inference_url wins when
//!                                  primary still points at OpenHuman after migration
//! "ollama:<model>[@<temp>]"      → local Ollama at config.local_ai.base_url
//! "lmstudio:<model>[@<temp>]"    → local LM Studio
//! "mlx:<model>[@<temp>]"         → local MLX-compatible server
//! "local-openai:<model>[@<temp>]"→ generic local OpenAI-compatible
//! "<slug>:<model>[@<temp>]"      → cloud_providers entry keyed by slug;
//!                                  builds OpenAiCompatibleProvider (Bearer) or
//!                                  Anthropic flavour depending on auth_style.
//! ```
//!
//! The optional `@<temp>` suffix pins a per-workload temperature override on
//! the built provider. The model id sent upstream never includes the suffix.
//!
//! Unknown slugs and missing-creds configurations produce actionable errors.

use crate::openhuman::config::schema::cloud_providers::AuthStyle;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use crate::openhuman::inference::provider::claude_agent_sdk::subprocess::ClaudeAgentSdkProvider;
use crate::openhuman::inference::provider::compatible::{
    AuthStyle as CompatAuthStyle, OpenAiCompatibleProvider,
};
use crate::openhuman::inference::provider::openai_codex::{
    openai_codex_client_version, openai_codex_user_agent, resolve_openai_codex_routing,
    OPENAI_CODEX_ACCOUNT_HEADER, OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_ORIGINATOR_HEADER,
};
use crate::openhuman::inference::provider::openhuman_backend::OpenHumanBackendProvider;
use crate::openhuman::inference::provider::traits::Provider;
use crate::openhuman::inference::provider::ProviderRuntimeOptions;

/// Sentinel meaning "use the OpenHuman backend session JWT".
pub const PROVIDER_OPENHUMAN: &str = "openhuman";
/// Prefix for Ollama-local providers: `"ollama:<model>"`.
pub const OLLAMA_PROVIDER_PREFIX: &str = "ollama:";
/// Prefix for LM Studio-local providers: `"lmstudio:<model>"`.
pub const LM_STUDIO_PROVIDER_PREFIX: &str = "lmstudio:";
/// Prefix for MLX-compatible local providers: `"mlx:<model>"`.
pub const MLX_PROVIDER_PREFIX: &str = "mlx:";
/// Prefix for OMLX local providers: `"omlx:<model>"`.
pub const OMLX_PROVIDER_PREFIX: &str = "omlx:";
/// Prefix for generic local OpenAI-compatible providers: `"local-openai:<model>"`.
pub const LOCAL_OPENAI_PROVIDER_PREFIX: &str = "local-openai:";
/// Prefix for the Claude Agent SDK subprocess provider: `"claude_agent_sdk:<model>"`.
pub const CLAUDE_AGENT_SDK_PREFIX: &str = "claude_agent_sdk:";
/// Sentinel for the Claude Agent SDK provider without a model suffix.
pub const CLAUDE_AGENT_SDK_PROVIDER: &str = "claude_agent_sdk";
/// Sentinel returned when a user has expressed custom/BYOK inference intent
/// (via a non-openhuman `inference_url`) but no matching `cloud_providers`
/// entry was found. Passed through `provider_for_role` and caught early in
/// `create_chat_provider_from_string` to produce a clear configuration error
/// instead of silently routing through the managed OpenHuman backend.
pub const BYOK_INCOMPLETE_SENTINEL: &str = "__byok_incomplete__";

fn is_abstract_tier_model(model: &str) -> bool {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
        MODEL_REASONING_V1, MODEL_VISION_V1,
    };
    // No dedicated constant for the summarization tier yet; keep the literal
    // in sync with the tier name used by the summarizer sub-agent.
    const MODEL_SUMMARIZATION_V1: &str = "summarization-v1";
    let trimmed = model.trim();
    trimmed == MODEL_REASONING_V1
        || trimmed == MODEL_REASONING_QUICK_V1
        || trimmed == MODEL_CHAT_V1
        || trimmed == MODEL_AGENTIC_V1
        || trimmed == MODEL_CODING_V1
        || trimmed == MODEL_VISION_V1
        || trimmed == MODEL_SUMMARIZATION_V1
}

/// Auth-profile storage key for a slug-keyed provider.
///
/// New writes use `"provider:<slug>"`. Lookups also try the bare `<slug>`
/// as a legacy fallback (old configs stored keys as e.g. `"openai:default"`).
pub fn auth_key_for_slug(slug: &str) -> String {
    format!("provider:{slug}")
}

/// Resolve a model hint (e.g. `"hint:reasoning"`) or tier name to the
/// concrete model string that the provider router would use — without
/// constructing the actual provider.  Returns the provider-string prefix
/// (e.g. `"openai"`) concatenated with the model when a BYOK provider is
/// active, or the bare tier name for the managed OpenHuman backend.
pub fn resolve_model_for_hint(hint_or_tier: &str, config: &Config) -> String {
    let hint_to_tier: &[(&str, &str)] = &[
        ("reasoning", crate::openhuman::config::MODEL_REASONING_V1),
        ("chat", crate::openhuman::config::MODEL_CHAT_V1),
        ("agentic", crate::openhuman::config::MODEL_AGENTIC_V1),
        ("coding", crate::openhuman::config::MODEL_CODING_V1),
        ("vision", crate::openhuman::config::MODEL_VISION_V1),
        ("summarization", "summarization-v1"),
    ];
    let tier_to_role: &[(&str, &str)] = &[
        (crate::openhuman::config::MODEL_REASONING_V1, "reasoning"),
        (crate::openhuman::config::MODEL_CHAT_V1, "chat"),
        (crate::openhuman::config::MODEL_REASONING_QUICK_V1, "chat"),
        (crate::openhuman::config::MODEL_AGENTIC_V1, "agentic"),
        (crate::openhuman::config::MODEL_CODING_V1, "coding"),
        (crate::openhuman::config::MODEL_VISION_V1, "vision"),
        ("summarization-v1", "summarization"),
    ];

    let (tier, role) = if let Some(hint_key) = hint_or_tier.strip_prefix("hint:") {
        let tier = hint_to_tier
            .iter()
            .find(|(k, _)| *k == hint_key)
            .map(|(_, v)| *v)
            .unwrap_or(hint_or_tier);
        let role = tier_to_role
            .iter()
            .find(|(k, _)| *k == tier)
            .map(|(_, v)| *v)
            .unwrap_or(hint_key);
        (tier, role)
    } else {
        let role = tier_to_role
            .iter()
            .find(|(k, _)| *k == hint_or_tier)
            .map(|(_, v)| *v)
            .unwrap_or("chat");
        (hint_or_tier, role)
    };

    let provider_string = provider_for_role(role, config);
    let ps = provider_string.trim();
    if ps.is_empty() || ps == "cloud" || ps == PROVIDER_OPENHUMAN || ps == BYOK_INCOMPLETE_SENTINEL
    {
        tier.to_string()
    } else if let Some(idx) = ps.find(':') {
        let model_with_temp = &ps[idx + 1..];
        let (model, _) = split_model_and_temperature(model_with_temp);
        model
    } else {
        ps.to_string()
    }
}

/// Return whether `model` is a recognized OpenHuman backend tier name.
///
/// Used to guard against stale `default_model` values (e.g. set by older UI
/// versions) that the backend would reject with HTTP 400.  The known tiers are
/// the constants in `crate::openhuman::config`; the four `hint:*` strings that
/// `make_openhuman_backend` actually translates are also accepted.  An
/// unrecognized `hint:*` value is intentionally rejected so the factory falls
/// back to the platform default instead of forwarding an untranslated string
/// to the backend.
pub(crate) fn is_known_openhuman_tier(model: &str) -> bool {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
        MODEL_REASONING_V1, MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
    };
    matches!(
        model,
        MODEL_REASONING_V1
            | MODEL_CHAT_V1
            | MODEL_AGENTIC_V1
            | MODEL_CODING_V1
            | MODEL_REASONING_QUICK_V1
            | MODEL_SUMMARIZATION_V1
            | MODEL_VISION_V1
            | "hint:reasoning"
            | "hint:chat"
            | "hint:agentic"
            | "hint:coding"
            | "hint:summarization"
            | "hint:vision"
    )
}

/// Per-tier vision (image-input) capability for the managed OpenHuman backend.
///
/// The remote managed backend (`api.tinyhumans.ai`) does not advertise per-tier
/// capabilities, so the core maintains this map itself. Accepts both the tier
/// constants and their `hint:*` forms (callers may pass either pre- or
/// post-resolution).
///
/// `reasoning-v1` is multimodal; the rest return `false` — flip an individual
/// arm to `true` once that tier is confirmed multimodal on the backend. This is
/// the **only** place to change managed-model vision; BYOK/custom models are
/// handled separately by the user-set `model_registry.vision` flag
/// ([`crate::openhuman::inference::model_context::model_vision_enabled`]).
pub(crate) fn oh_tier_supports_vision(model: &str) -> bool {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
        MODEL_REASONING_V1, MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
    };
    match model {
        MODEL_REASONING_V1 | "hint:reasoning" => true,
        // Dedicated multimodal tier — the managed backend serves this with the
        // vision flag enabled. This is what the vision sub-agent rides on.
        MODEL_VISION_V1 | "hint:vision" => true,
        MODEL_CHAT_V1 | "hint:chat" => false,
        MODEL_REASONING_QUICK_V1 => false,
        MODEL_AGENTIC_V1 | "hint:agentic" => false,
        MODEL_CODING_V1 | "hint:coding" => false,
        MODEL_SUMMARIZATION_V1 | "hint:summarization" => false,
        _ => false,
    }
}

/// Return the configured provider string for a named workload role.
///
/// Empty / `"cloud"` resolves through BYOK fallback first for the three
/// chat-tier roles (`chat`, `reasoning`, `coding`), then `primary_cloud`.
/// When a BYOK cloud provider is detected on any workload, unset chat-tier
/// routes inherit it rather than silently falling back to the managed backend.
///
/// Only `chat`, `reasoning`, and `coding` participate in BYOK inheritance.
/// Background workloads (`memory`, `embeddings`, `heartbeat`, `learning`,
/// `subconscious`) and the `agentic` workload always fall through to
/// `primary_cloud` — they use tier-specific models that BYOK providers don't
/// understand, and their providers are configured independently.
///
/// For backwards compatibility, a legacy external `inference_url` takes
/// precedence when `primary_cloud` still points at OpenHuman because
/// migration 1→2 preserved the URL as a custom provider entry but older
/// configs did not explicitly set per-workload routes.
pub fn provider_for_role(role: &str, config: &Config) -> String {
    let opt = match role {
        "chat" => config.chat_provider.as_deref(),
        "reasoning" => config.reasoning_provider.as_deref(),
        "agentic" => config.agentic_provider.as_deref(),
        "coding" => config.coding_provider.as_deref(),
        // Tier-specific multimodal model; like `agentic` it is NOT part of the
        // chat-tier BYOK inheritance below — when unset it falls through to
        // `primary_cloud` (→ managed `vision-v1`).
        "vision" => config.vision_provider.as_deref(),
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
        // BYOK inheritance is scoped to the three chat-tier roles only.
        // Background workloads (memory, embeddings, heartbeat, learning,
        // subconscious) and the agentic workload must stay on the managed
        // backend — they use tier-specific models that BYOK providers don't
        // understand, and their providers are configured separately.
        if matches!(role, "chat" | "reasoning" | "coding") {
            if let Some(byok) = resolve_byok_fallback_provider_string(config) {
                log::debug!(
                    "[providers][byok-fallback] role={} inheriting BYOK provider string={}",
                    role,
                    byok
                );
                return byok;
            }
        }

        // Diagnostic: when the user has a local provider configured for chat
        // but this background workload is falling through to cloud, emit a
        // warning so it's visible in logs (no silent fallback).
        if !matches!(role, "chat" | "reasoning" | "coding") {
            if let Some(chat) = config.chat_provider.as_deref() {
                if crate::openhuman::inference::local::profile::is_local_provider_string(chat) {
                    log::info!(
                        "[providers][local-fallback] role={} using managed backend (chat is \
                         local '{}' but background workloads require cloud — set \
                         {}_provider explicitly to override)",
                        role,
                        chat,
                        role
                    );
                }
            }
        }

        resolve_primary_cloud_provider_string(config)
    } else {
        s.to_string()
    }
}

/// #3767: Whether the OpenHuman managed-credits gate should be bypassed for a
/// single workload role.
///
/// Returns true when `role` resolves (via [`provider_for_role`]) to a non-managed
/// provider the user funds themselves — a BYO cloud key (incl. OpenAI OAuth), a
/// local runtime, or claude-code — with usable credentials. When the role is on
/// the OpenHuman managed backend, or a BYO route has no usable key, it returns
/// false (the gate stays on; #3767: "BYO key present but invalid/unverified →
/// still gated").
///
/// The gate is evaluated per-tier so the UI can check the tier the user actually
/// selected: the chat header's "Quick" mode runs on the `chat` tier and
/// "Reasoning" mode on the `reasoning` tier, so each is checked respectively.
/// These per-role results are surfaced under `credits_bypass` in the
/// client-config snapshot. Tiers that stay managed and run anyway surface the
/// per-call `USER_INSUFFICIENT_CREDITS` (402) error reactively.
pub fn role_bypasses_managed_credits(role: &str, config: &Config) -> bool {
    let resolved = provider_for_role(role, config);
    let r = resolved.trim();
    let is_managed =
        r.is_empty() || r == "cloud" || r == PROVIDER_OPENHUMAN || r == BYOK_INCOMPLETE_SENTINEL;
    let usable_byo = !is_managed && route_has_usable_credentials(r, config);
    log::debug!(
        "[billing] role_bypasses_managed_credits role={role} resolved={resolved} \
         is_managed={is_managed} usable_byo={usable_byo}"
    );
    usable_byo
}

/// True when a resolved chat-tier provider string can actually run on the
/// user's own funding: local runtimes / claude-code carry their own creds; a
/// concrete cloud slug requires a non-empty stored key. Managed/sentinel
/// strings are filtered by the caller and never reach here as "usable".
fn route_has_usable_credentials(resolved: &str, config: &Config) -> bool {
    let r = resolved.trim();
    // Local runtimes (ollama/lmstudio/mlx/local-openai) and the local CLI
    // delegates carry their own credentials / run on-device.
    if crate::openhuman::inference::local::profile::is_local_provider_string(r)
        || r.starts_with(crate::openhuman::inference::provider::claude_code::PROVIDER_PREFIX)
        || r == CLAUDE_AGENT_SDK_PROVIDER
        || r.starts_with(CLAUDE_AGENT_SDK_PREFIX)
    {
        return true;
    }
    // Concrete cloud slug "<slug>:<model>" — require a usable stored key.
    if let Some((slug, _)) = r.split_once(':') {
        let slug = slug.trim();
        if !slug.is_empty() {
            // Don't silently swallow auth-store / OAuth lookup failures — a
            // transient Err would otherwise keep the credits gate on for a
            // valid BYO setup with no diagnostics. Log and treat as not-usable.
            match lookup_key_for_slug(slug, config) {
                Ok(key) => {
                    let usable = !key.trim().is_empty();
                    log::debug!(
                        "[billing] route_has_usable_credentials slug={slug} usable={usable}"
                    );
                    return usable;
                }
                Err(e) => {
                    log::debug!(
                        "[billing] route_has_usable_credentials slug={slug} lookup_error={e}"
                    );
                    return false;
                }
            }
        }
    }
    false
}

/// Find the first BYOK cloud provider string configured across all workload
/// routes, skipping local providers and managed-backend sentinels
/// ("openhuman", "cloud", empty).
///
/// Returns `None` when no BYOK cloud provider is configured, in which case
/// the caller should fall through to `resolve_primary_cloud_provider_string`.
///
/// Priority order: chat → reasoning → agentic → coding (user-facing workloads
/// first so the most prominent setting wins for unset background workloads).
pub(crate) fn resolve_byok_fallback_provider_string(config: &Config) -> Option<String> {
    let candidates = [
        config.chat_provider.as_deref(),
        config.reasoning_provider.as_deref(),
        config.agentic_provider.as_deref(),
        config.coding_provider.as_deref(),
    ];
    for candidate in candidates.iter().flatten() {
        let s = candidate.trim();
        if s.is_empty() || s == "cloud" || s == PROVIDER_OPENHUMAN {
            continue;
        }
        // Skip local providers — they are not suitable fallbacks for agentic
        // or background workloads that run on the managed backend.
        if s.starts_with(OLLAMA_PROVIDER_PREFIX)
            || s.starts_with(LM_STUDIO_PROVIDER_PREFIX)
            || s.starts_with(MLX_PROVIDER_PREFIX)
            || s.starts_with(OMLX_PROVIDER_PREFIX)
            || s.starts_with(LOCAL_OPENAI_PROVIDER_PREFIX)
        {
            continue;
        }
        // Any remaining non-empty string with a colon is a BYOK cloud slug.
        if s.contains(':') {
            log::debug!(
                "[providers][byok-fallback] resolve_byok_fallback found candidate={}",
                s
            );
            return Some(s.to_string());
        }
    }
    None
}

/// Test-only seam: inject a mock chat `Provider` so e2e tests can drive the
/// autonomous run paths (`spawn_workflow_run_background`, the task dispatcher)
/// with a scripted LLM and no network. Process-global because those runs are
/// detached `tokio::spawn`s — a thread/task-local would not reach them.
///
/// Because it is global, tests that install an override MUST run serially
/// and clear it via the returned guard. Inert in production: the check below
/// is gated on `cfg(test)` or the off-by-default `e2e-test-support` feature,
/// so the override is never consulted in shipped builds.
#[cfg(any(test, feature = "e2e-test-support"))]
pub mod test_provider_override {
    use super::Provider;
    use crate::openhuman::inference::provider::traits::{
        ChatRequest, ChatResponse, ProviderCapabilities,
    };
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex, OnceLock};

    static OVERRIDE: OnceLock<Mutex<Option<Arc<dyn Provider>>>> = OnceLock::new();
    fn cell() -> &'static Mutex<Option<Arc<dyn Provider>>> {
        OVERRIDE.get_or_init(|| Mutex::new(None))
    }

    pub(crate) fn current() -> Option<Arc<dyn Provider>> {
        cell().lock().unwrap().clone()
    }

    /// Install a mock provider; the returned guard clears it on drop.
    #[must_use]
    pub fn install(provider: Arc<dyn Provider>) -> InstallGuard {
        *cell().lock().unwrap() = Some(provider);
        InstallGuard
    }
    pub struct InstallGuard;
    impl Drop for InstallGuard {
        fn drop(&mut self) {
            *cell().lock().unwrap() = None;
        }
    }

    /// Thin delegating wrapper so the factory can hand out a fresh
    /// `Box<dyn Provider>` backed by the shared mock `Arc` — one mock instance
    /// serves the orchestrator AND the inner workflow run, routing by prompt
    /// content. Forwards the methods the turn engine actually calls; the rest
    /// use the trait defaults (which read back through `capabilities`).
    pub(crate) struct ProviderHandle(pub Arc<dyn Provider>);

    #[async_trait]
    impl Provider for ProviderHandle {
        fn capabilities(&self) -> ProviderCapabilities {
            self.0.capabilities()
        }
        async fn chat_with_system(
            &self,
            system_prompt: Option<&str>,
            message: &str,
            model: &str,
            temperature: f64,
        ) -> anyhow::Result<String> {
            self.0
                .chat_with_system(system_prompt, message, model, temperature)
                .await
        }
        async fn chat(
            &self,
            request: ChatRequest<'_>,
            model: &str,
            temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            self.0.chat(request, model, temperature).await
        }
    }
}

/// Build a `(Provider, model)` for the given workload role.
pub fn create_chat_provider(
    role: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    // Test-only: a scripted mock provider injected by an e2e test wins over
    // anything config-derived. Gated on cfg(test) / the off-by-default
    // `e2e-test-support` feature; never consulted in shipped builds.
    #[cfg(any(test, feature = "e2e-test-support"))]
    if let Some(p) = test_provider_override::current() {
        return Ok((
            Box::new(test_provider_override::ProviderHandle(p)),
            "mock-model".to_string(),
        ));
    }

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

    // Fail-closed: BYOK intent was detected upstream but no matching provider
    // entry was found. Surface a clear configuration error instead of silently
    // routing through the managed OpenHuman backend.
    if p == BYOK_INCOMPLETE_SENTINEL {
        let inference_url = config
            .inference_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("<unset>");
        anyhow::bail!(
            "[chat-factory] BYOK_INCOMPLETE: inference_url is set to a custom/direct endpoint \
             ({inference_url}) but no matching cloud_providers entry was found for role '{role}'. \
             To complete BYOK setup add a cloud_providers entry whose endpoint matches \
             {inference_url} (or use a workload-specific route). \
             To use the OpenHuman managed backend instead, clear inference_url from config."
        );
    }

    // Empty / legacy "cloud" sentinel → primary cloud target.
    if p.is_empty() || p == "cloud" {
        let resolved = resolve_primary_cloud_provider_string(config);
        return create_chat_provider_from_string(role, &resolved, config);
    }

    if p == PROVIDER_OPENHUMAN {
        return make_openhuman_backend(role, config);
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

    if let Some(model_with_temp) =
        p.strip_prefix(crate::openhuman::inference::provider::claude_code::PROVIDER_PREFIX)
    {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if temperature_override.is_some() {
            log::warn!(
                "[providers][chat-factory] claude-code provider: per-model temperature override \
                 is accepted but not yet wired through to the CLI — the @<temp> suffix is ignored"
            );
        }
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty model — \
                 use 'claude-code:<model-id>'",
                p,
                role
            );
        }
        let workspace =
            crate::openhuman::inference::provider::claude_code::workspace_dir_from_config(config);
        log::debug!(
            "[providers][chat-factory] building claude-code CLI provider model={} workspace={}",
            model,
            workspace.display()
        );
        let provider =
            crate::openhuman::inference::provider::claude_code::ClaudeCodeProvider::from_env(
                model.clone(),
                workspace,
                config.action_dir.clone(),
            )?;
        let p_box: Box<dyn Provider> = Box::new(provider);
        return Ok((p_box, model));
    }

    if let Some(model_with_temp) = p.strip_prefix(OLLAMA_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty model — \
                 use 'ollama:<model-id>'",
                p,
                role
            );
        }
        return make_ollama_provider(&model, temperature_override, config);
    }

    if let Some(model_with_temp) = p.strip_prefix(LM_STUDIO_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty model — \
                 use 'lmstudio:<model-id>'",
                p,
                role
            );
        }
        return make_lm_studio_provider(&model, temperature_override, config);
    }

    if let Some(model_with_temp) = p.strip_prefix(MLX_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty model — \
                 use 'mlx:<model-id>'",
                p,
                role
            );
        }
        return make_mlx_provider(&model, temperature_override, config);
    }

    if let Some(model_with_temp) = p.strip_prefix(OMLX_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty model — \
                 use 'omlx:<model-id>'",
                p,
                role
            );
        }
        return make_omlx_provider(&model, temperature_override, config);
    }

    if let Some(model_with_temp) = p.strip_prefix(LOCAL_OPENAI_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty model — \
                 use 'local-openai:<model-id>'",
                p,
                role
            );
        }
        return make_local_openai_provider(&model, temperature_override, config);
    }

    if p == CLAUDE_AGENT_SDK_PROVIDER || p.starts_with(CLAUDE_AGENT_SDK_PREFIX) {
        let model = if let Some(m) = p.strip_prefix(CLAUDE_AGENT_SDK_PREFIX) {
            m.trim().to_string()
        } else {
            config.claude_agent_sdk.default_model.clone()
        };
        tracing::debug!(
            "[providers][chat-factory] creating claude_agent_sdk provider model={}",
            model
        );
        let provider = ClaudeAgentSdkProvider::new(config.claude_agent_sdk.clone());
        return Ok((Box::new(provider), model));
    }

    // New grammar: "<slug>:<model>[@<temp>]"
    if let Some(colon_pos) = p.find(':') {
        let slug = p[..colon_pos].trim();
        let (model, temperature_override) = split_model_and_temperature(&p[colon_pos + 1..]);

        if slug.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' for role '{}' has an empty slug",
                p,
                role
            );
        }

        return make_cloud_provider_by_slug(role, slug, &model, temperature_override, config);
    }

    // No colon: might be a bare legacy type string (e.g. "openai"). Try as
    // slug lookup with empty model — gives a clear "no entry" error rather
    // than an opaque parse failure.
    anyhow::bail!(
        "[chat-factory] unrecognised provider string '{}' for role '{}'. \
         Valid forms: openhuman, ollama:<model>, lmstudio:<model>, mlx:<model>, omlx:<model>, \
         local-openai:<model>, claude_agent_sdk, claude_agent_sdk:<model>, <slug>:<model>. \
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

/// Build a local-runtime provider without applying the custom-provider session gate.
///
/// Used by setup/probe flows that need to validate an endpoint before the
/// workload routing layer is fully configured. This still routes through the
/// same standardized compatible-provider implementation as the main factory.
pub(crate) fn create_local_chat_provider_from_string(
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    let p = provider.trim();
    log::debug!(
        "[providers][chat-factory] create_local_chat_provider_from_string provider={}",
        p
    );

    if let Some(model_with_temp) = p.strip_prefix(OLLAMA_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' has an empty model — use 'ollama:<model-id>'",
                p
            );
        }
        log::debug!(
            "[providers][chat-factory] local:ollama model={} temp={:?}",
            model,
            temperature_override
        );
        return make_ollama_provider(&model, temperature_override, config);
    }

    if let Some(model_with_temp) = p.strip_prefix(LM_STUDIO_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' has an empty model — use 'lmstudio:<model-id>'",
                p
            );
        }
        log::debug!(
            "[providers][chat-factory] local:lmstudio model={} temp={:?}",
            model,
            temperature_override
        );
        return make_lm_studio_provider(&model, temperature_override, config);
    }

    if let Some(model_with_temp) = p.strip_prefix(MLX_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' has an empty model — use 'mlx:<model-id>'",
                p
            );
        }
        return make_mlx_provider(&model, temperature_override, config);
    }

    if let Some(model_with_temp) = p.strip_prefix(OMLX_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' has an empty model — use 'omlx:<model-id>'",
                p
            );
        }
        return make_omlx_provider(&model, temperature_override, config);
    }

    if let Some(model_with_temp) = p.strip_prefix(LOCAL_OPENAI_PROVIDER_PREFIX) {
        let (model, temperature_override) = split_model_and_temperature(model_with_temp);
        if model.is_empty() {
            anyhow::bail!(
                "[chat-factory] provider string '{}' has an empty model — use 'local-openai:<model-id>'",
                p
            );
        }
        return make_local_openai_provider(&model, temperature_override, config);
    }

    anyhow::bail!(
        "[chat-factory] '{}' is not a supported local provider string. Valid local forms: \
         ollama:<model>, lmstudio:<model>, mlx:<model>, omlx:<model>, local-openai:<model>",
        p
    );
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Build the OpenHuman backend provider (session-JWT auth).
///
/// `role` is the workload name (e.g. `"chat"`, `"vision"`). The managed backend
/// otherwise derives its model from `config.default_model` (which defaults to the
/// non-vision `chat-v1` tier), so a tier-specific workload whose per-workload
/// provider is unset would silently inherit the global default. For the `vision`
/// workload that mismatch is fatal: an unset `vision_provider` would resolve to
/// `chat-v1`, `model_supports_vision` would report `false`, and the turn engine
/// would strip every attached image — leaving the managed vision sub-agent blind.
/// Pin `vision` to the dedicated multimodal `vision-v1` tier so the managed
/// default path keeps working without requiring the user to set `vision_provider`.
fn make_openhuman_backend(
    role: &str,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    let model = if role == "vision" {
        crate::openhuman::config::MODEL_VISION_V1.to_string()
    } else {
        config
            .default_model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| "reasoning-v1".to_string())
    };
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
    // canonical tier names.  Unrecognised `hint:*` strings (e.g. `hint:reaction`
    // for lightweight models) are forwarded as-is — the backend is authoritative
    // over which hint values it accepts, and the web-chat model_override path
    // uses these verbatim.  Only non-hint strings that are not a known canonical
    // tier (stale `default_model` values written by older UI versions, e.g.
    // "deepseek-v4-pro", "claude-opus-4-7") fall back to the platform default.
    let model = match model.strip_prefix("hint:") {
        Some("reasoning") => crate::openhuman::config::MODEL_REASONING_V1.to_string(),
        Some("chat") => crate::openhuman::config::MODEL_CHAT_V1.to_string(),
        Some("agentic") => crate::openhuman::config::MODEL_AGENTIC_V1.to_string(),
        Some("coding") => crate::openhuman::config::MODEL_CODING_V1.to_string(),
        Some("summarization") => crate::openhuman::config::MODEL_SUMMARIZATION_V1.to_string(),
        Some("vision") => crate::openhuman::config::MODEL_VISION_V1.to_string(),
        Some(_) => {
            // Unrecognised hint — forward verbatim; the backend decides validity.
            model
        }
        None => {
            if is_known_openhuman_tier(&model) {
                model
            } else {
                log::warn!(
                    "[providers][chat-factory] model '{}' is not a recognized OpenHuman \
                     backend tier (valid: reasoning-v1, chat-v1, agentic-v1, coding-v1, \
                     reasoning-quick-v1, summarization-v1, vision-v1); falling back to '{}'",
                    model,
                    crate::openhuman::config::MODEL_REASONING_V1,
                );
                crate::openhuman::config::MODEL_REASONING_V1.to_string()
            }
        }
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
    // AgentBox marketplace containers run headless with no desktop
    // `app-session` JWT — the deployment is operator-controlled and ships its
    // own GMI MaaS credentials via `GMI_*` env vars. The session gate exists to
    // stop an *unregistered desktop user* from routing every workload at a
    // custom provider; that threat model doesn't apply here, so bypass it.
    // Without this, every `/run` job would fail `SESSION_EXPIRED` before
    // reaching GMI (the startup path stores only `provider:gmi-maas`).
    if crate::openhuman::agentbox::agentbox_mode_enabled() {
        log::debug!(
            "[chat-factory] AgentBox mode — bypassing app-session gate for custom provider"
        );
        return Ok(());
    }
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

fn resolve_primary_cloud_provider_string(config: &Config) -> String {
    let primary = config
        .primary_cloud
        .as_deref()
        .and_then(|id| config.cloud_providers.iter().find(|entry| entry.id == id));

    if primary.is_some_and(is_openhuman_cloud_entry) {
        if let Some(legacy) = legacy_custom_inference_provider_string(config) {
            return legacy;
        }
        // Primary is explicitly OpenHuman but inference_url points at a custom
        // endpoint with no matching provider entry — this is a half-migrated BYOK
        // config. Fail closed so the user sees an actionable error rather than
        // silently routing through the managed backend.
        if has_custom_inference_intent(config) {
            log::debug!(
                "[providers][chat-factory] BYOK intent detected (host={}) \
                 but no matching cloud_providers entry found; returning fail-closed sentinel",
                redact_inference_url(config.inference_url.as_deref())
            );
            return BYOK_INCOMPLETE_SENTINEL.to_string();
        }
    }

    if let Some(entry) = primary {
        return cloud_entry_provider_string(entry, config);
    }

    // No explicit primary configured. If inference_url signals custom intent but
    // no matching provider entry exists, fail closed instead of falling back to
    // the managed backend.
    legacy_custom_inference_provider_string(config).unwrap_or_else(|| {
        if has_custom_inference_intent(config) {
            log::debug!(
                "[providers][chat-factory] BYOK intent detected (host={}) \
                 with no primary_cloud and no matching provider entry; returning fail-closed sentinel",
                redact_inference_url(config.inference_url.as_deref())
            );
            BYOK_INCOMPLETE_SENTINEL.to_string()
        } else {
            PROVIDER_OPENHUMAN.to_string()
        }
    })
}

/// Extract the host portion of an inference URL for safe logging.
///
/// Returns the host (e.g. `"api.example.com"`) so log lines are grep-friendly
/// without exposing tokens or credentials that may appear in query-string or
/// path components of a bearer-auth URL (e.g. `"https://host/v1?key=…"`).
/// Falls back to `"<redacted>"` when the URL cannot be parsed or is absent.
fn redact_inference_url(url: Option<&str>) -> &str {
    url.and_then(|u| {
        // Minimal host extraction: find the authority after "://".
        let after_scheme = u.find("://").map(|i| &u[i + 3..])?;
        // Authority ends at '/', '?', '#', or end-of-string.
        let host_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let authority = &after_scheme[..host_end];
        // Strip optional "user:pass@" and port.
        let host = authority
            .rfind('@')
            .map_or(authority, |i| &authority[i + 1..]);
        let host = host.rfind(':').map_or(host, |i| &host[..i]);
        if host.is_empty() {
            None
        } else {
            Some(host)
        }
    })
    .unwrap_or("<redacted>")
}

/// Return `true` when the config contains a non-openhuman `inference_url`,
/// indicating the user intends custom/BYOK routing rather than the managed
/// backend.
fn has_custom_inference_intent(config: &Config) -> bool {
    config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .is_some_and(|url| !looks_like_openhuman_backend(url))
}

fn legacy_custom_inference_provider_string(config: &Config) -> Option<String> {
    let inference_url = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())?;

    if looks_like_openhuman_backend(inference_url) {
        return None;
    }

    let normalized_inference = normalize_endpoint_for_compare(inference_url);
    config
        .cloud_providers
        .iter()
        .find(|entry| {
            !is_openhuman_cloud_entry(entry)
                && normalize_endpoint_for_compare(&entry.endpoint) == normalized_inference
        })
        .map(|entry| cloud_entry_provider_string(entry, config))
}

/// Resolve the slug of the cloud-provider entry that represents the legacy
/// direct-inference route — the entry whose endpoint matches the configured
/// custom `inference_url`.
///
/// Top-level `config.api_key` was historically paired with `inference_url`
/// for direct endpoint routing, so it is scoped to this single provider. The
/// `lookup_key_for_slug` fallback uses this to avoid leaking the global key to
/// any other provider slug whose auth-profile lookup returned empty.
fn legacy_inference_slug(config: &Config) -> Option<&str> {
    let inference_url = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())?;

    if looks_like_openhuman_backend(inference_url) {
        return None;
    }

    let normalized_inference = normalize_endpoint_for_compare(inference_url);
    config
        .cloud_providers
        .iter()
        .find(|entry| {
            !is_openhuman_cloud_entry(entry)
                && normalize_endpoint_for_compare(&entry.endpoint) == normalized_inference
        })
        .map(|entry| entry.slug.as_str())
}

fn cloud_entry_provider_string(
    entry: &crate::openhuman::config::schema::cloud_providers::CloudProviderCreds,
    config: &Config,
) -> String {
    if is_openhuman_cloud_entry(entry) {
        return PROVIDER_OPENHUMAN.to_string();
    }

    let model = entry
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .or_else(|| {
            config
                .default_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
        })
        .unwrap_or(crate::openhuman::config::DEFAULT_MODEL);

    format!("{}:{model}", entry.slug)
}

fn is_openhuman_cloud_entry(
    entry: &crate::openhuman::config::schema::cloud_providers::CloudProviderCreds,
) -> bool {
    entry.slug == PROVIDER_OPENHUMAN
        || matches!(entry.auth_style, AuthStyle::OpenhumanJwt)
        || looks_like_openhuman_backend(&entry.endpoint)
}

fn normalize_endpoint_for_compare(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn looks_like_openhuman_backend(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let without_scheme = lower.split("://").nth(1).unwrap_or(&lower);
    let authority = without_scheme.split('/').next().unwrap_or("");
    let host = authority.split('@').next_back().unwrap_or(authority);
    let host_no_port = host.split(':').next().unwrap_or(host);
    matches!(
        host_no_port,
        "api.openhuman.ai" | "api.tinyhumans.ai" | "staging-api.tinyhumans.ai" | "openhuman"
    ) || host_no_port.ends_with(".openhuman.ai")
        || host_no_port.ends_with(".tinyhumans.ai")
}

/// Parse a `<model>[@<temp>]` tail into `(model, override)`.
///
/// Tolerates whitespace around the components. Returns `temperature = None`
/// when the suffix is absent or unparseable — the model text is taken as-is.
fn split_model_and_temperature(raw: &str) -> (String, Option<f64>) {
    let trimmed = raw.trim();
    if let Some(at_pos) = trimmed.rfind('@') {
        let head = trimmed[..at_pos].trim();
        let tail = trimmed[at_pos + 1..].trim();
        if !head.is_empty() {
            if let Ok(parsed) = tail.parse::<f64>() {
                if parsed.is_finite() {
                    return (head.to_string(), Some(parsed));
                }
            }
        }
    }
    (trimmed.to_string(), None)
}

/// Build an Ollama local provider.
fn make_ollama_provider(
    model: &str,
    temperature_override: Option<f64>,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    use crate::openhuman::inference::local::profile::LocalProviderKind;

    let base_url = crate::openhuman::inference::local::ollama_base_url_from_config(config);
    let normalized_base_url = base_url.trim_end_matches('/').trim_end_matches("/v1");
    // Ollama exposes an OpenAI-compatible endpoint at /v1.
    let endpoint = format!("{normalized_base_url}/v1");
    let num_ctx = config.local_ai.num_ctx;
    log::info!(
        "[providers][chat-factory] building ollama provider model={} endpoint_host={} \
         temp_override={:?} num_ctx={:?}",
        model,
        redact_endpoint(&endpoint),
        temperature_override,
        num_ctx,
    );
    // Ollama does not expose the Responses API (/v1/responses) — passing
    // `false` prevents a guaranteed-404 fallback attempt and the Sentry
    // noise it would generate (TAURI-RUST-59Y).
    //
    // Ollama also rejects the OpenAI-style `tools` parameter for many models
    // (HTTP 400 "unsupported parameter: tools"), so we disable
    // `native_tool_calling` on the provider directly. The agent harness
    // then embeds tool specs in the system prompt and parses tool calls
    // out of the response text — a format any chat model can follow.
    // Skills that depend on tool invocations now work over Ollama
    // (sub-issue 3 of #3098).
    let provider = OpenAiCompatibleProvider::new_no_responses_fallback(
        "ollama",
        &endpoint,
        None,
        CompatAuthStyle::None,
    )
    .with_temperature_unsupported_models(config.temperature_unsupported_models.clone())
    .with_temperature_override(temperature_override)
    .with_native_tool_calling(false)
    .with_vision(false)
    .with_ollama_num_ctx(num_ctx)
    .with_local_provider_kind(LocalProviderKind::Ollama);
    Ok((Box::new(provider), model.to_string()))
}

/// Build an LM Studio local provider.
fn make_lm_studio_provider(
    model: &str,
    temperature_override: Option<f64>,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    use crate::openhuman::inference::local::profile::LocalProviderKind;

    let endpoint = crate::openhuman::inference::local::lm_studio::lm_studio_base_url(config);
    let api_key = config.local_ai.api_key.as_deref().unwrap_or("");
    log::info!(
        "[providers][chat-factory] building lmstudio provider model={} endpoint_host={} temp_override={:?}",
        model,
        redact_endpoint(&endpoint),
        temperature_override
    );
    // LM Studio does not expose the Responses API — same rationale as Ollama.
    let auth = if api_key.trim().is_empty() {
        CompatAuthStyle::None
    } else {
        CompatAuthStyle::Bearer
    };
    let provider = OpenAiCompatibleProvider::new_no_responses_fallback(
        "lmstudio",
        &endpoint,
        if api_key.trim().is_empty() {
            None
        } else {
            Some(api_key)
        },
        auth,
    )
    .with_temperature_unsupported_models(config.temperature_unsupported_models.clone())
    .with_temperature_override(temperature_override)
    .with_native_tool_calling(false)
    .with_vision(false)
    .with_local_provider_kind(LocalProviderKind::LmStudio);
    Ok((Box::new(provider), model.to_string()))
}

/// Build an MLX-compatible local provider.
///
/// MLX servers (e.g. `mlx_lm.server`) expose an OpenAI-compatible endpoint.
/// Default URL: `http://127.0.0.1:8080/v1` (override via `MLX_SERVER_URL` env
/// or `local_ai.base_url` when provider is set to "mlx").
fn make_mlx_provider(
    model: &str,
    temperature_override: Option<f64>,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    use crate::openhuman::inference::local::profile::{LocalProviderKind, MLX_PROFILE};

    let endpoint = std::env::var("MLX_SERVER_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| config.local_ai.base_url.clone())
        .unwrap_or_else(|| MLX_PROFILE.default_base_url.to_string());
    log::info!(
        "[providers][chat-factory] building mlx provider model={} endpoint_host={} temp_override={:?}",
        model,
        redact_endpoint(&endpoint),
        temperature_override
    );
    let provider = OpenAiCompatibleProvider::new_no_responses_fallback(
        "mlx",
        &endpoint,
        None,
        CompatAuthStyle::None,
    )
    .with_temperature_unsupported_models(config.temperature_unsupported_models.clone())
    .with_temperature_override(temperature_override)
    .with_native_tool_calling(false)
    .with_vision(false)
    .with_local_provider_kind(LocalProviderKind::Mlx);
    Ok((Box::new(provider), model.to_string()))
}

/// Build an OMLX local provider.
///
/// OMLX servers expose an OpenAI v1-compatible endpoint and require a Bearer API key.
/// Default URL: `http://127.0.0.1:8000/v1` (override via `OMLX_SERVER_URL` env
/// or `local_ai.base_url` when provider is set to "omlx").
fn make_omlx_provider(
    model: &str,
    temperature_override: Option<f64>,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    use crate::openhuman::inference::local::profile::{LocalProviderKind, OMLX_PROFILE};

    let endpoint = std::env::var("OMLX_SERVER_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| config.local_ai.base_url.clone())
        .unwrap_or_else(|| OMLX_PROFILE.default_base_url.to_string());
    let api_key = config.local_ai.api_key.as_deref().unwrap_or("");
    if api_key.trim().is_empty() {
        log::warn!(
            "[providers][chat-factory] omlx: no api_key configured — OMLX requires a Bearer key; \
             requests will likely 401"
        );
    }
    log::info!(
        "[providers][chat-factory] building omlx provider model={} endpoint_host={} temp_override={:?}",
        model,
        redact_endpoint(&endpoint),
        temperature_override
    );
    let auth = if api_key.trim().is_empty() {
        CompatAuthStyle::None
    } else {
        CompatAuthStyle::Bearer
    };
    let provider = OpenAiCompatibleProvider::new_no_responses_fallback(
        "omlx",
        &endpoint,
        if api_key.trim().is_empty() {
            None
        } else {
            Some(api_key)
        },
        auth,
    )
    .with_temperature_unsupported_models(config.temperature_unsupported_models.clone())
    .with_temperature_override(temperature_override)
    .with_native_tool_calling(false)
    .with_vision(false)
    .with_local_provider_kind(LocalProviderKind::Omlx);
    Ok((Box::new(provider), model.to_string()))
}

/// Build a generic local OpenAI-compatible provider.
///
/// Points at any local server that speaks the OpenAI chat-completions API
/// (llama.cpp, vLLM, text-generation-inference, etc.).
/// Default URL: `http://127.0.0.1:8080/v1` (override via `LOCAL_OPENAI_URL`
/// env or `local_ai.base_url`).
fn make_local_openai_provider(
    model: &str,
    temperature_override: Option<f64>,
    config: &Config,
) -> anyhow::Result<(Box<dyn Provider>, String)> {
    use crate::openhuman::inference::local::profile::{LocalProviderKind, LOCAL_OPENAI_PROFILE};

    let endpoint = std::env::var("LOCAL_OPENAI_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| config.local_ai.base_url.clone())
        .unwrap_or_else(|| LOCAL_OPENAI_PROFILE.default_base_url.to_string());
    let api_key = config.local_ai.api_key.as_deref().unwrap_or("");
    log::info!(
        "[providers][chat-factory] building local-openai provider model={} endpoint_host={} temp_override={:?}",
        model,
        redact_endpoint(&endpoint),
        temperature_override
    );
    let auth = if api_key.trim().is_empty() {
        CompatAuthStyle::None
    } else {
        CompatAuthStyle::Bearer
    };
    let provider = OpenAiCompatibleProvider::new_no_responses_fallback(
        "local-openai",
        &endpoint,
        if api_key.trim().is_empty() {
            None
        } else {
            Some(api_key)
        },
        auth,
    )
    .with_temperature_unsupported_models(config.temperature_unsupported_models.clone())
    .with_temperature_override(temperature_override)
    .with_native_tool_calling(false)
    .with_vision(false)
    .with_local_provider_kind(LocalProviderKind::LocalOpenai);
    Ok((Box::new(provider), model.to_string()))
}

/// Look up a `cloud_providers` entry by slug and build the provider.
fn make_cloud_provider_by_slug(
    role: &str,
    slug: &str,
    model: &str,
    temperature_override: Option<f64>,
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
    let mut effective_model = if model.trim().is_empty() {
        entry.default_model.clone().unwrap_or_default()
    } else {
        model.to_string()
    };

    // Guard: if effective_model is still empty after fallback, bail with an
    // actionable error. Sending an empty model string to providers like
    // nvidia-nim causes a 400 "model field is required" — a confusing error
    // that obscures the real cause (missing model in the provider string or
    // unset default_model on the config entry).
    // See https://github.com/tinyhumansai/openhuman/issues/2784.
    //
    // OpenhumanJwt entries are exempt: they always delegate to
    // make_openhuman_backend which derives the model from config.default_model,
    // ignoring whatever effective_model we computed here.
    if entry.auth_style != AuthStyle::OpenhumanJwt && effective_model.trim().is_empty() {
        log::warn!(
            "[nvidia-nim][chat-factory] role={} slug={} resolved to empty model — \
             provider string must include a model id (e.g. '{}:<model-id>') or \
             set default_model on the cloud_providers entry",
            role,
            slug,
            slug,
        );
        anyhow::bail!(
            "[chat-factory] no model configured: role '{}' resolved to an empty model id for slug '{}'. \
             Include a model in the provider string (e.g. '{slug}:<model-id>') or \
             set default_model on the cloud_providers entry for slug '{slug}'.",
            role,
            slug,
        );
    }

    if entry.auth_style != AuthStyle::OpenhumanJwt && is_abstract_tier_model(&effective_model) {
        if let Some(default_model) = entry
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty() && !is_abstract_tier_model(m))
        {
            log::info!(
                "[providers][chat-factory] role={} slug={} remapping abstract model {} -> {}",
                role,
                slug,
                effective_model,
                default_model
            );
            effective_model = default_model.to_string();
        } else {
            anyhow::bail!(
                "[chat-factory] model '{}' is an abstract tier for role '{}', \
                 but cloud provider slug '{}' has no concrete default_model configured. \
                 Set cloud_providers[].default_model to a provider-native model id (e.g. deepseek-v4-pro).",
                effective_model,
                role,
                slug
            );
        }
    }

    log::info!(
        "[providers][chat-factory] role={} slug={} model={} endpoint_host={}",
        role,
        slug,
        effective_model,
        redact_endpoint(&entry.endpoint)
    );

    let key = lookup_key_for_slug(slug, config)?;
    let openai_codex_routing = resolve_openai_codex_routing(config, slug, &entry.endpoint, &key)
        .map_err(anyhow::Error::msg)?;

    let unsupported = &config.temperature_unsupported_models;
    match entry.auth_style {
        AuthStyle::Anthropic => {
            let p = make_openai_compatible_provider_with_config(
                slug,
                &entry.endpoint,
                &key,
                CompatAuthStyle::Anthropic,
                unsupported,
                temperature_override,
                true,
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
            make_openhuman_backend(role, config)
        }
        AuthStyle::None => {
            let p = make_openai_compatible_provider_with_config(
                slug,
                &entry.endpoint,
                "",
                CompatAuthStyle::None,
                unsupported,
                temperature_override,
                true,
            )?;
            Ok((p, effective_model))
        }
        AuthStyle::Bearer => {
            log::info!(
                "[providers][chat-factory] role={} slug={} codex_oauth={} endpoint_host={} account_id_header={}",
                role,
                slug,
                openai_codex_routing.using_oauth,
                redact_endpoint(&openai_codex_routing.endpoint),
                openai_codex_routing.account_id.is_some()
            );
            let mut provider = OpenAiCompatibleProvider::new(
                slug,
                &openai_codex_routing.endpoint,
                (!key.trim().is_empty()).then_some(key.as_str()),
                CompatAuthStyle::Bearer,
            )
            .with_temperature_unsupported_models(unsupported.to_vec())
            .with_temperature_override(temperature_override);
            if let Some(account_id) = openai_codex_routing.account_id.as_deref() {
                provider = provider.with_extra_header(OPENAI_CODEX_ACCOUNT_HEADER, account_id);
            }
            if openai_codex_routing.using_oauth {
                provider = provider
                    .with_extra_header(OPENAI_CODEX_ORIGINATOR_HEADER, OPENAI_CODEX_ORIGINATOR)
                    .with_user_agent(openai_codex_user_agent())
                    .with_extra_query_param("client_version", openai_codex_client_version())
                    .with_responses_api_primary();
            }
            let p: Box<dyn Provider> = Box::new(provider);
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
        match crate::openhuman::inference::openai_oauth::lookup_openai_bearer_token(config) {
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

/// Build an `OpenAiCompatibleProvider` with the given auth style.
fn make_openai_compatible_provider(
    endpoint: &str,
    api_key: &str,
    auth_style: CompatAuthStyle,
) -> anyhow::Result<Box<dyn Provider>> {
    make_openai_compatible_provider_with_config(
        "cloud",
        endpoint,
        api_key,
        auth_style,
        &[],
        None,
        true,
    )
}

/// Build an `OpenAiCompatibleProvider` with auth style, temperature
/// suppression list from config, and an optional per-workload temperature
/// override (extracted from the provider string's `@<temp>` suffix).
///
/// `supports_responses_fallback` controls whether a 404 on the chat
/// completions endpoint triggers an automatic retry against `/v1/responses`.
/// Local providers (Ollama, LM Studio) do not expose the Responses API, so
/// passing `false` for them prevents a guaranteed-404 secondary request and
/// the Sentry noise it would generate (TAURI-RUST-59Y).
fn make_openai_compatible_provider_with_config(
    provider_name: &str,
    endpoint: &str,
    api_key: &str,
    auth_style: CompatAuthStyle,
    temperature_unsupported_models: &[String],
    temperature_override: Option<f64>,
    supports_responses_fallback: bool,
) -> anyhow::Result<Box<dyn Provider>> {
    let key = if api_key.trim().is_empty() {
        None
    } else {
        Some(api_key)
    };
    log::debug!(
        "[providers][chat-factory] building compatible provider name={} endpoint_host={} responses_fallback={} temp_override={:?}",
        provider_name,
        redact_endpoint(endpoint),
        supports_responses_fallback,
        temperature_override
    );
    let provider = if supports_responses_fallback {
        OpenAiCompatibleProvider::new(provider_name, endpoint, key, auth_style)
    } else {
        OpenAiCompatibleProvider::new_no_responses_fallback(
            provider_name,
            endpoint,
            key,
            auth_style,
        )
    };
    Ok(Box::new(
        provider
            .with_temperature_unsupported_models(temperature_unsupported_models.to_vec())
            .with_temperature_override(temperature_override),
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
#[path = "factory_tests.rs"]
mod factory_tests;
