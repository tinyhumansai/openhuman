//! Classifier for **provider configuration-rejection** errors.
//!
//! When OpenHuman talks to a user-configured custom cloud endpoint
//! (`custom_openai` → DeepSeek / OpenRouter / Moonshot / …) the upstream
//! API rejects requests whose model id or sampling params it doesn't
//! understand:
//!
//! - `"The supported API model names are deepseek-v4-pro or
//!   deepseek-v4-flash, but you passed reasoning-v1."` (#2079 — an
//!   OpenHuman abstract tier alias leaked to a provider that only speaks
//!   its own native ids)
//! - `"Model 'deepseek-v4-pro' is not available. Use GET
//!   /openai/v1/models to list available models."` (#2202)
//! - `"invalid temperature: only 1 is allowed for this model"` (#2076 —
//!   Moonshot Kimi K2)
//! - `"The model \`gpt-5.5\` does not exist or you do not have access to
//!   it."` / `"model_not_found"` (stale model pin)
//!
//! These are **deterministic user-configuration state**, not bugs the
//! maintainers can act on: the user pointed OpenHuman at a custom
//! provider with a model / temperature that provider does not accept. The
//! remediation is "fix the model or routing in Settings", which the UI
//! surfaces. Yet every agent turn produces a fresh Sentry event
//! (OPENHUMAN-TAURI-WJ / -QW / -HB / -NH — 88 + 146 + 39 events). This is
//! the same class as budget-exhaustion ([`super::billing_error`]) and
//! must be demoted from Sentry to an info log the same way.
//!
//! ## Provider-aware polarity (important)
//!
//! The phrases below are emitted by **third-party upstream APIs**
//! (DeepSeek / OpenRouter / Moonshot). The OpenHuman hosted backend
//! resolves tier aliases natively and never emits "supported API model
//! names are deepseek-…" or "invalid temperature: only 1 is allowed" — so
//! the phrase set is intrinsically scoped to custom providers. The
//! HTTP-layer wrapper [`super::ops::is_provider_config_rejection_http`]
//! additionally guards on `provider != openhuman_backend::PROVIDER_LABEL`
//! so a model-rejection from our **own** backend (which would be a real
//! regression we sent it a bad request) still reaches Sentry. The
//! message-only predicate is consumed by
//! [`crate::core::observability::expected_error_kind`] for the
//! re-reported error that escapes the provider layer and is raised again
//! by `agent.run_single` / `web_channel.run_chat_task`.
//!
//! Keep the list deliberately tight: a false positive demotes a real
//! provider/backend bug to an info log.

/// Returns true if a provider error body indicates the request was
/// rejected because of the user's model / parameter **configuration**
/// (unknown model id, abstract tier leaked to a custom provider,
/// model-specific temperature constraint), as opposed to a transient
/// failure or a server bug.
///
/// Case-insensitive substring match. See the module docs for the polarity
/// contract and the OPENHUMAN-TAURI Sentry issues each phrase drops.
pub fn is_provider_config_rejection_message(body: &str) -> bool {
    const PHRASES: &[&str] = &[
        // #2079 — an OpenHuman abstract tier alias (`reasoning-v1`,
        // `chat-v1`, …) reached a custom provider that lists its own
        // native ids back at us.
        "supported api model names are",
        // #2202 — OpenAI-compatible "unknown model" body. The
        // `/openai/v1/models` remediation hint is the stable, unique
        // anchor (the quoted model id varies per user).
        "/openai/v1/models",
        // OpenAI / OpenRouter stale-pin shape (`claude-opus-4-7`,
        // `gpt-5.5`, …) — model removed or no access.
        "does not exist or you do not have access",
        "model_not_found",
        // #2076 — Moonshot Kimi K2 only accepts `temperature: 1`.
        "invalid temperature",
        "only 1 is allowed for this model",
        // Our own actionable error once a proper tier→model resolution
        // is in place (keeps this classifier stable across that fix).
        "is an abstract tier",
    ];

    let lower = body.to_ascii_lowercase();
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_real_sentry_bodies() {
        // The exact upstream bodies from OPENHUMAN-TAURI-WJ / -QW / -HB
        // / -NH and the stale-pin family.
        for body in [
            "The supported API model names are deepseek-v4-pro or deepseek-v4-flash, but you passed reasoning-v1.",
            "Model 'deepseek-v4-pro' is not available. Use GET /openai/v1/models to list available models.",
            "Model 'claude-opus-4-7' is not available. Use GET /openai/v1/models to list available models.",
            "invalid temperature: only 1 is allowed for this model",
            "The model `gpt-5.5` does not exist or you do not have access to it.",
            r#"{"error":{"message":"model not found","code":"model_not_found"}}"#,
            "Model 'reasoning-v1' is an abstract tier — configure a concrete model for your custom provider",
        ] {
            assert!(
                is_provider_config_rejection_message(body),
                "{body:?} must classify as a provider config-rejection user-state"
            );
        }
    }

    #[test]
    fn detection_is_case_insensitive() {
        assert!(is_provider_config_rejection_message(
            "INVALID TEMPERATURE: ONLY 1 IS ALLOWED FOR THIS MODEL"
        ));
        assert!(is_provider_config_rejection_message(
            "The Supported API Model Names Are gpt-4o or gpt-4o-mini"
        ));
    }

    #[test]
    fn ignores_transient_and_server_and_unrelated() {
        // Must NOT demote: transient/server failures and generic 4xx
        // that carry no config-rejection signal — those stay Sentry
        // actionable. (A real backend bug must not be silenced.)
        for body in [
            "Internal server error",
            "503 Service Unavailable",
            "Bad request: missing field",
            "rate limit exceeded, retry after 1s",
            "insufficient budget — add credits",
            "",
        ] {
            assert!(
                !is_provider_config_rejection_message(body),
                "{body:?} must NOT classify as a provider config-rejection"
            );
        }
    }
}
