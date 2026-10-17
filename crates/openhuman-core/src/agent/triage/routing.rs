//! Local-vs-remote provider resolver for triage turns.
//!
//! ## What this does
//!
//! [`resolve_provider`] always builds the remote provider. Local AI is never
//! used for chat triage — the local path has been removed to guarantee that
//! a triage turn never errors due to Ollama unavailability.
//!
//! `ResolvedProvider.used_local` is preserved for telemetry compatibility but
//! is always `false`.

use std::sync::Arc;

use crate::config::Config;
use crate::inference::provider::{self, INFERENCE_BACKEND_ID};

/// Workload role every triage arm runs under. Triage classification is a
/// short chat-shaped call, so it follows the user's `chat` route.
const TRIAGE_ROLE: &str = "chat";

/// The concrete provider + metadata that [`crate::agent::triage::evaluator::run_triage`]
/// should use for this particular triage turn.
pub struct ResolvedProvider {
    /// Model source for this arm. Production sources are crate-native.
    pub turn_model_source: crate::agent::tinyagents::TurnModelSource,
    /// Provider name token — always `"openhuman"` (remote backend).
    /// Kept for telemetry / observability compat with the previous two-path design.
    pub provider_name: String,
    /// Model identifier — the concrete string the turn
    /// (`TurnSpec::run`) will hand to the provider.
    pub model: String,
    /// Always `false` — local AI is never used for triage.
    /// Preserved so existing telemetry subscribers that read this field do not
    /// need code changes.
    pub used_local: bool,
}

// ── Public API ──────────────────────────────────────────────────────────

/// Inner half of [`resolve_provider`] that takes an already-loaded
/// [`Config`]. Exposed for tests and for the evaluator's retry path.
pub async fn resolve_provider_with_config(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> anyhow::Result<ResolvedProvider> {
    tracing::debug!(
        runtime_enabled = config.local_ai.runtime_enabled,
        "[triage::routing] resolving provider (always remote)"
    );
    build_remote_provider(ctx, config)
}

/// Build the local-arm provider for the tiered fallback chain (issue
/// #1257). Returns `None` when local AI is disabled or no chat model
/// is configured — callers (`evaluator::run_triage`) skip straight to
/// `Deferred` in that case.
///
/// The returned source is crate-native and targets the configured local
/// inference base (Ollama by default, overridable via
/// `OPENHUMAN_LOCAL_INFERENCE_URL`).
pub fn build_local_provider_with_config(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> Option<ResolvedProvider> {
    let local_cfg = &config.local_ai;
    if !local_cfg.runtime_enabled {
        tracing::debug!("[triage::routing] local arm disabled (runtime_enabled=false)");
        return None;
    }
    if local_cfg.chat_model_id.trim().is_empty() {
        tracing::debug!("[triage::routing] local arm skipped (no chat_model_id configured)");
        return None;
    }

    let override_base = std::env::var("OPENHUMAN_LOCAL_INFERENCE_URL")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());
    let provider_kind = local_cfg.provider.trim().to_ascii_lowercase();
    let use_openai_compat = override_base.is_some()
        || matches!(
            provider_kind.as_str(),
            "llamacpp" | "llama-server" | "custom_openai"
        );

    let (label, provider_string) = if use_openai_compat {
        let label = if provider_kind == "custom_openai" {
            "custom_openai"
        } else {
            "llamacpp"
        };
        (label, format!("local-openai:{}", local_cfg.chat_model_id))
    } else {
        ("ollama", format!("ollama:{}", local_cfg.chat_model_id))
    };
    tracing::debug!(
        provider = %label,
        model = %local_cfg.chat_model_id,
        "[triage::routing] resolved local fallback provider"
    );
    Some(ResolvedProvider {
        turn_model_source: crate::agent::tinyagents::TurnModelSource::new_crate_native_from_string(
            ctx,
            TRIAGE_ROLE,
            provider_string,
            Arc::new(config.clone()),
        ),
        provider_name: label.to_string(),
        model: local_cfg.chat_model_id.clone(),
        used_local: true,
    })
}

/// Whether a resolved provider string targets a local CLI delegate
/// (`claude_agent_sdk` / `claude-code:<model>`). These carry their own auth and
/// spawn a local process, so — like the local HTTP runtimes — they must never be
/// the provider for a triage turn (#1257). Kept separate from
/// `is_local_provider_string`, which only classifies the local HTTP runtimes.
fn is_local_cli_route(provider_string: &str) -> bool {
    use crate::inference::provider::factory::{CLAUDE_AGENT_SDK_PREFIX, CLAUDE_AGENT_SDK_PROVIDER};
    use tinyagents_harness::providers::claude_code;
    let s = provider_string.trim();
    s == CLAUDE_AGENT_SDK_PROVIDER
        || s.starts_with(CLAUDE_AGENT_SDK_PREFIX)
        || s.starts_with(claude_code::PROVIDER_PREFIX)
}

// ── Provider builder ────────────────────────────────────────────────────

/// Build the remote provider for a triage turn, routed through the
/// [`TRIAGE_ROLE`] (`chat`) workload.
///
/// The managed model id comes from `make_openhuman_backend` →
/// [`managed_tier_for_role`]`("chat")` (i.e. `hint:chat`), the same registry
/// the agent harness uses — NOT from `default_model`, so one place pins the
/// managed model.
///
/// #1257 invariant — *triage never goes local*: when the resolved
/// `chat_provider` is a local runtime (Ollama/LM Studio/MLX/…) or a
/// BYOK-incomplete sentinel, we force the managed backend so a trigger never
/// errors because a local model is down. Only a concrete BYOK **cloud** route is
/// honoured as-is. A build failure also falls back to the managed backend.
fn build_remote_provider(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> anyhow::Result<ResolvedProvider> {
    use crate::inference::provider::factory::{
        create_chat_model_from_string_with_model_id, PROVIDER_OPENHUMAN,
    };
    use tinyinference_local::profile::is_local_provider_string;

    let resolved = provider::provider_for_role(TRIAGE_ROLE, config);
    let r = resolved.trim();

    // #1257: triage must never depend on a local model/CLI being up, and a
    // half-configured BYOK route must not error a trigger — force the managed
    // backend in all those cases. `is_local_provider_string` only covers the
    // local HTTP runtimes (Ollama/LM Studio/MLX/local-openai), so the local CLI
    // delegates (`claude_agent_sdk`, `claude-code:<model>`) are excluded
    // explicitly here (Codex P2) — otherwise they'd be treated as BYOK cloud and
    // triage could hang on an unauthenticated/missing CLI.
    let force_managed = is_local_provider_string(r)
        || is_local_cli_route(r)
        || r == provider::BYOK_INCOMPLETE_SENTINEL;
    let effective = if force_managed { PROVIDER_OPENHUMAN } else { r };
    if force_managed {
        tracing::info!(
            resolved = %r,
            "[triage::routing] chat workload not usable for triage (local/incomplete) — \
             forcing managed backend (#1257: triage never goes local)"
        );
    }

    // Build through the per-workload factory: managed routes resolve their model
    // id via `make_openhuman_backend` → `managed_tier_for_role`, BYOK cloud routes
    // via the slug's configured model.
    let build = |provider_string: &str| -> anyhow::Result<ResolvedProvider> {
        let (_chat_model, model) = create_chat_model_from_string_with_model_id(
            TRIAGE_ROLE,
            provider_string,
            config,
            config.default_temperature,
        )?;
        let provider_name = if provider_string == PROVIDER_OPENHUMAN {
            INFERENCE_BACKEND_ID.to_string()
        } else {
            provider_string
                .split(':')
                .next()
                .unwrap_or(provider_string)
                .to_string()
        };
        Ok(ResolvedProvider {
            turn_model_source:
                crate::agent::tinyagents::TurnModelSource::new_crate_native_from_string(
                    ctx,
                    TRIAGE_ROLE,
                    provider_string,
                    Arc::new(config.clone()),
                ),
            provider_name,
            model,
            used_local: false,
        })
    };

    match build(effective) {
        Ok(rp) => {
            tracing::debug!(
                provider = %rp.provider_name,
                model = %rp.model,
                "[triage::routing] resolved remote provider via chat workload"
            );
            Ok(rp)
        }
        Err(err) => {
            tracing::warn!(
                resolved = %effective,
                error = %err,
                "[triage::routing] chat workload provider build failed — \
                 falling back to managed backend"
            );
            build(PROVIDER_OPENHUMAN)
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
