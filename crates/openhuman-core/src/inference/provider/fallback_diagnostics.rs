//! OpenHuman wording for native workload-router fallback diagnostics.

use tinyagents_registry::router::FallbackDiagnostics;

fn role_capability_label(role: &str) -> Option<&'static str> {
    match role.trim() {
        "vision" => Some("vision (image input)"),
        "embeddings" => Some("embeddings"),
        "memory" | "summarization" => Some("summarization"),
        "agentic" | "burst" => Some("agentic tool use"),
        _ => None,
    }
}

fn role_phrase(role: &str) -> String {
    role_capability_label(role)
        .map(str::to_owned)
        .unwrap_or_else(|| role.trim().to_owned())
}

/// OpenHuman's branded notice for the native router's implicit cloud fallback.
pub(crate) fn cloud_fallback_notice(
    diagnostic: &FallbackDiagnostics,
    local_chat_provider: &str,
) -> String {
    format!(
        "{} is running via your managed/cloud provider ('{}') because your local chat model \
         ('{}') does not serve this workload. Set {}_provider explicitly in Connections to \
         override.",
        capitalize_first(&role_phrase(&diagnostic.role)),
        diagnostic.fallback_route.trim(),
        local_chat_provider.trim(),
        diagnostic.override_role,
    )
}

/// Branded missing-credentials copy for a fallback selected by the native router.
pub(crate) fn missing_provider_credentials_message(
    diagnostic: &FallbackDiagnostics,
    slug: &str,
    local_chat_provider: Option<&str>,
) -> String {
    let slug = slug.trim();
    let role = role_phrase(&diagnostic.role);
    match local_chat_provider.filter(|local| !local.trim().is_empty()) {
        Some(local) => format!(
            "No usable credentials for '{slug}', which OpenHuman selected for the {role} workload. \
             Your chat model is local ('{}') and does not serve this workload, so it fell back \
             to your cloud provider — but '{slug}' has no API key configured. Add a key for \
             '{slug}' in Connections → LLM, set {}_provider to a provider that is configured, \
             or enable the managed OpenHuman backend.",
            local.trim(),
            diagnostic.override_role,
        ),
        None => format!(
            "No usable credentials for '{slug}', which OpenHuman selected for the {role} workload. \
             Add a key for '{slug}' in Connections → LLM, set {}_provider to a provider that is \
             configured, or enable the managed OpenHuman backend.",
            diagnostic.override_role,
        ),
    }
}

/// Actionable message for the vision pre-flight when a selected local model
/// cannot accept image input.
pub(crate) fn local_vision_unsupported_message(model: &str) -> String {
    format!(
        "The selected local model ('{}') does not support vision (image input). Switch to a \
         vision-capable model such as llava:7b (`ollama pull llava:7b`), or set vision_provider \
         in Connections → LLM to a provider that supports images.",
        model.trim()
    )
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "fallback_diagnostics_tests.rs"]
mod tests;
