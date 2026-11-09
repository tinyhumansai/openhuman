//! Context-window projections and user-configured model capabilities.

/// Resolves a model-only context window using owner-provided static hints.
///
/// This projection uses a fresh native resolver, so it cannot borrow remembered
/// provider discovery from a route. Returns `None` when no owner or
/// managed-provider hint is available.
pub fn context_window_for_model(model: &str) -> Option<u64> {
    crate::inference::context_window::project_context_window_for_model(model, None).window
}

/// Resolve the synchronous window for a model on a specific provider route.
pub(crate) fn context_window_for_route(
    provider: &str,
    model: &str,
    config: &crate::config::Config,
) -> Option<u64> {
    crate::inference::context_window::project_context_window("chat", provider, model, config).window
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
/// - **Hosted models and role aliases** consult the registered provider's
///   capability hook; the backend does not advertise per-model capability.
/// - **Custom/BYOK models** consult the user-set `model_registry.vision` flag
///   ([`model_vision_enabled`]).
pub fn model_supports_vision(model: &str, config: &crate::config::Config) -> bool {
    crate::inference::registry::provider_model_supports_vision(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        model,
    ) || model_vision_enabled(model, config)
}

pub(crate) fn model_supports_vision_in(
    extensions: Option<&crate::core::slots::Extensions>,
    model: &str,
    config: &crate::config::Config,
) -> bool {
    crate::inference::registry::provider_model_supports_vision_for(
        extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        model,
    ) || model_vision_enabled(model, config)
}

#[cfg(test)]
#[path = "model_context_tests.rs"]
mod tests;
