//! Known model context-window sizes for pre-inference budgeting.
//!
//! Provider `/models` responses may include `context_length` / `context_window`,
//! but the agent harness must enforce limits **before** the first dispatch —
//! otherwise long histories produce upstream `400 Bad Request` errors when usage
//! metadata is not yet available.

use crate::config::{legacy_tier_role, MODEL_MANAGED_DEFAULT};

/// Conservative default for OpenHuman abstract tier models (tokens).
const TIER_LARGE_CONTEXT: u64 = 200_000;
/// Reasoning tier — backed by a 1M-context model.
const TIER_REASONING_CONTEXT: u64 = 1_000_000;
const TIER_STANDARD_CONTEXT: u64 = 128_000;
const TIER_LOCAL_CONTEXT: u64 = 8_192;

/// DeepSeek v4 Flash window (~1M tokens) — the managed default model
/// (`MODEL_MANAGED_DEFAULT`) and the backing of the retired flash tiers. It
/// must reflect the real model's capacity.
const TIER_FLASH_CONTEXT: u64 = 1_000_000;

/// Resolve the context window (in tokens) for a model id or OpenHuman tier alias.
///
/// Returns the static model estimate. Call [`context_window_for_route`] when
/// the provider route is known; provider-discovered values are stored per
/// route and model. Thread usage prefers its persisted per-turn value before
/// consulting either lookup.
///
/// Returns `None` when the model is unknown — callers should skip pre-dispatch
/// trimming rather than guess.
pub fn context_window_for_model(model: &str) -> Option<u64> {
    static_context_window_for_model(model)
}

/// Resolve the synchronous window for a model on a specific provider route.
pub(crate) fn context_window_for_route(
    provider: &str,
    model: &str,
    config: &crate::config::Config,
) -> Option<u64> {
    crate::inference::context_window::config_override(model, config)
        .or_else(|| crate::inference::context_window::remembered_window(provider, model))
        .or_else(|| static_context_window_for_model(model))
}

/// The static guess for a model's window: tier aliases, then the cost catalog,
/// then the generic id-pattern hints. These never override a provider-reported
/// window; [`crate::inference::context_window`] consults them last.
pub(crate) fn static_context_window_for_model(model: &str) -> Option<u64> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    if let Some(window) = tier_context_window(normalized) {
        return Some(window);
    }

    if let Some(price) = tinyagents_registry::pricing::lookup(normalized) {
        tracing::debug!(
            model = normalized,
            catalog_model = price.model_id,
            context_window = price.context_window,
            "[model_context] matched cost catalog row"
        );
        return Some(u64::from(price.context_window));
    }

    if let Some(window) = tinyinference_llm::model::context_window_for_model_id(normalized) {
        tracing::debug!(
            model = normalized,
            context_window = window,
            "[model_context] matched tinyagents model context hint"
        );
        return Some(window);
    }

    None
}

fn tier_context_window(model: &str) -> Option<u64> {
    if model == MODEL_MANAGED_DEFAULT {
        return Some(TIER_FLASH_CONTEXT);
    }
    // Role aliases and retired tier slugs: the windows the roles ran on before
    // every managed role collapsed onto the default model. Kept so an alias a
    // caller still holds budgets the way it used to.
    let role = model
        .strip_prefix("hint:")
        .or_else(|| legacy_tier_role(model))
        .unwrap_or("");
    match role {
        "reasoning" => Some(TIER_REASONING_CONTEXT),
        "agentic" | "coding" => Some(TIER_LARGE_CONTEXT),
        "burst" => Some(TIER_STANDARD_CONTEXT),
        "chat" | "summarization" => Some(TIER_FLASH_CONTEXT),
        _ if model == "chat" => Some(TIER_FLASH_CONTEXT),
        _ if model.starts_with("gemma") || model.contains(":1b") || model.contains("270m") => {
            Some(TIER_LOCAL_CONTEXT)
        }
        _ => None,
    }
}

/// Whether the model resolved for a chat hint/agent/profile accepts image input
/// according to the **user-configured** vision flag in `config.model_registry`.
///
/// This is the per-model override that lets a user mark a **custom / BYOK** model
/// as vision-capable (Settings → Advanced LLM → custom model → "Supports
/// vision"). Managed-backend models already advertise vision via
/// [`crate::inference::provider::Provider::supports_vision`]; this flag
/// covers OpenAI-compatible providers the backend can't introspect per-model.
/// Returns `false` for models the user has not flagged.
pub fn model_vision_enabled(model: &str, config: &crate::config::Config) -> bool {
    let normalized = model.trim();
    if normalized.is_empty() {
        return false;
    }
    let enabled = config
        .model_registry
        .iter()
        .any(|entry| entry.id == normalized && entry.vision);
    tracing::debug!(
        model = normalized,
        vision_enabled = enabled,
        "[model_context] resolved user-configured vision flag"
    );
    enabled
}

/// Whether a resolved model accepts image input. The single predicate shared by
/// the chat UI resolve and the server-side session/sub-agent gates.
///
/// - **Managed models and role aliases** consult the core-owned map
///   ([`crate::inference::provider::factory::oh_tier_supports_vision`]) —
///   the remote backend does not advertise per-model capability. The managed
///   default model and the `vision` / `reasoning` aliases are vision-capable.
/// - **Custom/BYOK models** consult the user-set `model_registry.vision` flag
///   ([`model_vision_enabled`]).
pub fn model_supports_vision(model: &str, config: &crate::config::Config) -> bool {
    crate::inference::provider::factory::oh_tier_supports_vision(model)
        || model_vision_enabled(model, config)
}

#[cfg(test)]
#[path = "model_context_tests.rs"]
mod tests;
