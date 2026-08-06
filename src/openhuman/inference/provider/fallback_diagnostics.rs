//! Actionable diagnostics for background-workload provider fallback (#5146 §2.1).
//!
//! ## The user-visible bug
//!
//! A user configures a local Ollama model for chat, attaches an image, and gets
//! `failed to read API key for slug 'anthropic'` — naming a provider they never
//! configured. The same shape appears for embeddings, memory and the other
//! background workloads.
//!
//! ## Why it happens, and why the routing itself is correct
//!
//! [`super::factory::provider_for_role`] deliberately routes the background
//! roles (`vision`, `embeddings`, `memory`, `heartbeat`, `learning`,
//! `subconscious`, `agentic`, `burst`) to the primary cloud provider when their
//! own route is unset: they run tier-specific models (`vision-v1`,
//! `summarization-v1`, …) that local runtimes and BYOK slugs do not serve. A
//! user on a local chat model with a managed subscription genuinely wants those
//! workloads on the cloud, so the fallback must stay.
//!
//! What was wrong is that the fallback was **unexplained**. When it landed on a
//! provider with no usable credentials, the failure surfaced as a raw slug-level
//! auth error with no hint that (a) this was a background role, (b) it fell back
//! because the local model cannot serve that capability, or (c) what to do about
//! it. That is the whole of the fix here: the routing decision is unchanged, the
//! *explanation* is not.
//!
//! Everything in this module is a pure string/predicate function so the wording
//! and the routing rules can be unit-tested without a live provider.
//!
//! Users who want no cloud egress at all already have
//! [`crate::openhuman::config::schema::PrivacyMode::LocalOnly`], enforced at the
//! inference chokepoint by `factory::enforce_local_only_inference`. This module
//! deliberately does not duplicate that gate.

/// Workload roles that inherit the primary cloud provider when their own route
/// is unset — the set [`super::factory::provider_for_role`] does **not** include
/// in chat-tier BYOK inheritance.
///
/// `chat`, `reasoning` and `coding` are absent on purpose: they inherit a
/// configured BYOK slug instead, so they never produce the confusion this
/// module exists to explain.
const CLOUD_FALLBACK_ROLES: &[&str] = &[
    "vision",
    "embeddings",
    "memory",
    "summarization",
    "heartbeat",
    "learning",
    "subconscious",
    "agentic",
    "burst",
];

/// Whether `role` falls through to the primary cloud provider when its own
/// route is unset.
pub(crate) fn role_falls_back_to_cloud(role: &str) -> bool {
    CLOUD_FALLBACK_ROLES.contains(&role.trim())
}

/// The capability a background role provides, phrased for a user-facing
/// sentence ("… does not support **vision**").
///
/// `None` for roles whose name is not a capability the user would recognise as
/// a model feature (`heartbeat`, `burst`, …); callers fall back to the role
/// name itself.
pub(crate) fn role_capability_label(role: &str) -> Option<&'static str> {
    match role.trim() {
        "vision" => Some("vision (image input)"),
        "embeddings" => Some("embeddings"),
        "memory" | "summarization" => Some("summarization"),
        "agentic" | "burst" => Some("agentic tool use"),
        _ => None,
    }
}

/// How a role is described in a sentence — the capability when it maps to one,
/// otherwise the bare role name.
fn role_phrase(role: &str) -> String {
    match role_capability_label(role) {
        Some(capability) => capability.to_string(),
        None => role.trim().to_string(),
    }
}

/// Explanation logged when a background role runs on the cloud because the
/// user's chat model is local and cannot serve that capability.
///
/// This is the informational, everything-is-working case: the user has a cloud
/// route available and the workload will succeed. It exists so the routing
/// decision is visible in logs and support transcripts rather than silent.
pub(crate) fn cloud_fallback_notice(
    role: &str,
    local_chat_provider: &str,
    resolved_provider: &str,
) -> String {
    format!(
        "{} is running via your managed/cloud provider ('{}') because your local chat model \
         ('{}') does not serve this workload. Set {}_provider explicitly in Connections to \
         override.",
        capitalize_first(&role_phrase(role)),
        resolved_provider.trim(),
        local_chat_provider.trim(),
        override_knob_for_role(role),
    )
}

/// The config knob a user sets to override a role's route.
///
/// `burst` has no knob of its own — it rides the agentic route — so it points
/// the user at `agentic_provider` rather than a setting that does not exist.
pub(crate) fn override_knob_for_role(role: &str) -> &str {
    match role.trim() {
        "burst" => "agentic",
        "summarization" => "memory",
        other => other,
    }
}

/// Actionable error for the failing case: a background role fell back to a
/// cloud slug whose credentials cannot be read.
///
/// Replaces the bare `failed to read API key for slug 'anthropic'` that named a
/// provider the user never chose. Mentions the local chat model when there is
/// one, because that is the missing half of the story.
pub(crate) fn missing_provider_credentials_message(
    role: &str,
    slug: &str,
    local_chat_provider: Option<&str>,
) -> String {
    let role = role.trim();
    let slug = slug.trim();
    match local_chat_provider {
        Some(local) if !local.trim().is_empty() => format!(
            "No usable credentials for '{slug}', which OpenHuman selected for the {} workload. \
             Your chat model is local ('{}') and does not serve this workload, so it fell back \
             to your cloud provider — but '{slug}' has no API key configured. Add a key for \
             '{slug}' in Connections → LLM, set {}_provider to a provider that is configured, \
             or enable the managed OpenHuman backend.",
            role_phrase(role),
            local.trim(),
            override_knob_for_role(role),
        ),
        _ => format!(
            "No usable credentials for '{slug}', which OpenHuman selected for the {} workload. \
             Add a key for '{slug}' in Connections → LLM, set {}_provider to a provider that is \
             configured, or enable the managed OpenHuman backend.",
            role_phrase(role),
            override_knob_for_role(role),
        ),
    }
}

/// Actionable message for the vision pre-flight: the active model cannot accept
/// images, so the caller should say so instead of stripping the image and
/// producing a confusing blind answer.
pub(crate) fn local_vision_unsupported_message(model: &str) -> String {
    format!(
        "The selected local model ('{}') does not support vision (image input). Switch to a \
         vision-capable model such as llava:7b (`ollama pull llava:7b`), or set vision_provider \
         in Connections → LLM to a provider that supports images.",
        model.trim()
    )
}

/// Pure pre-flight for image input: `Ok(())` when `model` accepts images,
/// `Err(actionable message)` when it does not.
///
/// Split from the call sites so the wording is unit-testable and identical
/// everywhere an image is about to be dropped.
pub(crate) fn vision_preflight(
    model: &str,
    config: &crate::openhuman::config::Config,
) -> Result<(), String> {
    if crate::openhuman::inference::model_context::model_supports_vision(model, config) {
        return Ok(());
    }
    Err(local_vision_unsupported_message(model))
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_tier_roles_do_not_fall_back_to_cloud() {
        // These inherit a BYOK slug instead; including them would attach the
        // "your local model can't serve this" explanation to the very roles the
        // user did configure locally.
        for role in ["chat", "reasoning", "coding"] {
            assert!(
                !role_falls_back_to_cloud(role),
                "{role} must not be treated as a cloud-fallback role"
            );
        }
    }

    #[test]
    fn background_roles_fall_back_to_cloud() {
        for role in [
            "vision",
            "embeddings",
            "memory",
            "summarization",
            "heartbeat",
            "learning",
            "subconscious",
            "agentic",
            "burst",
        ] {
            assert!(
                role_falls_back_to_cloud(role),
                "{role} must be treated as a cloud-fallback role"
            );
        }
    }

    #[test]
    fn unknown_role_does_not_fall_back() {
        assert!(!role_falls_back_to_cloud("not-a-role"));
        assert!(!role_falls_back_to_cloud(""));
    }

    #[test]
    fn role_falls_back_to_cloud_ignores_surrounding_whitespace() {
        assert!(role_falls_back_to_cloud("  vision  "));
    }

    #[test]
    fn burst_points_at_the_agentic_knob_that_actually_exists() {
        // There is no `burst_provider`; telling the user to set one would be a
        // dead end.
        assert_eq!(override_knob_for_role("burst"), "agentic");
        assert_eq!(override_knob_for_role("summarization"), "memory");
        assert_eq!(override_knob_for_role("vision"), "vision");
    }

    #[test]
    fn fallback_notice_names_capability_local_model_and_override() {
        let msg = cloud_fallback_notice("vision", "ollama:gemma3:1b", "anthropic:claude-sonnet-4");
        assert!(msg.contains("Vision (image input)"), "got: {msg}");
        assert!(msg.contains("ollama:gemma3:1b"), "got: {msg}");
        assert!(msg.contains("anthropic:claude-sonnet-4"), "got: {msg}");
        assert!(msg.contains("vision_provider"), "got: {msg}");
    }

    #[test]
    fn fallback_notice_for_role_without_capability_label_uses_role_name() {
        let msg = cloud_fallback_notice("heartbeat", "ollama:gemma3:1b", "openhuman");
        assert!(msg.contains("Heartbeat"), "got: {msg}");
        assert!(msg.contains("heartbeat_provider"), "got: {msg}");
    }

    #[test]
    fn missing_credentials_message_explains_the_local_fallback() {
        let msg =
            missing_provider_credentials_message("vision", "anthropic", Some("ollama:gemma3:1b"));
        // The whole point of #5146 §2.1: the user must learn *why* a provider
        // they never configured is being asked for a key.
        assert!(msg.contains("anthropic"), "got: {msg}");
        assert!(msg.contains("ollama:gemma3:1b"), "got: {msg}");
        assert!(msg.contains("vision (image input)"), "got: {msg}");
        assert!(msg.contains("Connections"), "got: {msg}");
        assert!(msg.contains("vision_provider"), "got: {msg}");
    }

    #[test]
    fn missing_credentials_message_without_local_chat_omits_the_local_clause() {
        let msg = missing_provider_credentials_message("embeddings", "openai", None);
        assert!(msg.contains("openai"), "got: {msg}");
        assert!(msg.contains("embeddings"), "got: {msg}");
        assert!(
            !msg.contains("Your chat model is local"),
            "must not claim a local chat model when there is none: {msg}"
        );
    }

    #[test]
    fn missing_credentials_message_treats_blank_local_provider_as_absent() {
        let msg = missing_provider_credentials_message("embeddings", "openai", Some("   "));
        assert!(
            !msg.contains("Your chat model is local"),
            "blank provider string is not a local model: {msg}"
        );
    }

    #[test]
    fn vision_unsupported_message_names_model_and_a_concrete_remedy() {
        let msg = local_vision_unsupported_message("gemma3:1b");
        assert!(msg.contains("gemma3:1b"), "got: {msg}");
        assert!(msg.contains("llava:7b"), "got: {msg}");
        assert!(msg.contains("vision_provider"), "got: {msg}");
    }

    #[test]
    fn vision_preflight_allows_a_vision_capable_model() {
        use crate::openhuman::config::schema::ModelRegistryEntry;
        let mut config = crate::openhuman::config::Config::default();
        config.model_registry = vec![ModelRegistryEntry {
            id: "llava:7b".into(),
            provider: "ollama".into(),
            cost_per_1m_output: 0.0,
            vision: true,
            ..Default::default()
        }];
        assert!(vision_preflight("llava:7b", &config).is_ok());
    }

    #[test]
    fn vision_preflight_rejects_a_text_only_model_with_an_actionable_message() {
        let config = crate::openhuman::config::Config::default();
        let err = vision_preflight("gemma3:1b", &config)
            .expect_err("a text-only model must fail the vision pre-flight");
        assert!(err.contains("gemma3:1b"), "got: {err}");
        assert!(err.contains("llava:7b"), "got: {err}");
    }

    #[test]
    fn vision_preflight_allows_the_managed_vision_tier() {
        // `vision-v1` is multimodal per `oh_tier_supports_vision`; the
        // pre-flight must not fire for the managed vision sub-agent.
        let config = crate::openhuman::config::Config::default();
        assert!(vision_preflight("vision-v1", &config).is_ok());
    }
}
