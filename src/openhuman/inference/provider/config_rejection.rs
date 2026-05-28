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
//! - `"This model is not available in your region."` (R1 — region-blocked
//!   model on a custom cloud provider)
//! - `"ModelNotAllowed"` (R4 — Doubao/ChatGLM model-allowlist enforcement)
//! - `"invalid_authentication_error"` (YC — user pasted a malformed /
//!   revoked API key into the provider config)
//! - `"This request requires more credits"` (S5 — OpenRouter `402` when
//!   the user's account is out of credits)
//! - `"Invalid model name passed in model="` (Y0 — litellm-style proxy
//!   rejecting a model id pre-routing)
//! - `"No active credentials for provider:"` (JN / KB — user hasn't
//!   plugged in their API key for the selected provider yet)
//! - `"litellm.BadRequestError"` (JK — litellm github_copilot proxy 400
//!   from a user OAuth/scope gap)
//! - `"not_found_error"` (J2 / J5 / J4 — litellm-compatible envelope
//!   `type` field carrying "model 'X' not found")
//!
//! These are **deterministic user-configuration state**, not bugs the
//! maintainers can act on: the user pointed OpenHuman at a custom
//! provider with a model / temperature / region / credential that
//! provider does not accept. The remediation is "fix the model, key, or
//! routing in Settings", which the UI surfaces. Yet every agent turn
//! produces a fresh Sentry event (OPENHUMAN-TAURI-WJ / -QW / -HB / -NH /
//! -R1 / -R4 / -YC / -S5 / -Y0 / -JN / -KB / -JK / -J2 / -J5 / -J4 —
//! ~250 additional events on top of the Wave 1-3 IDs). This is the
//! same class as budget-exhaustion ([`super::billing_error`]) and must
//! be demoted from Sentry to an info log the same way.
//!
//! ## Provider-aware polarity (important)
//!
//! Most of the phrases below are emitted by **third-party upstream APIs**
//! (DeepSeek / OpenRouter / Moonshot). The OpenHuman hosted backend
//! resolves tier aliases natively and never emits "supported API model
//! names are deepseek-…" or "invalid temperature: only 1 is allowed" — so
//! that phrase set is intrinsically scoped to custom providers. The
//! HTTP-layer wrapper [`super::ops::is_provider_config_rejection_http`]
//! polarity-guards those phrases on `provider !=
//! openhuman_backend::PROVIDER_LABEL` so a model-rejection from our
//! **own** backend that we did not expect (which would be a real
//! regression we sent it a bad request) still reaches Sentry. The
//! message-only predicate is consumed by
//! [`crate::core::observability::expected_error_kind`] for the
//! re-reported error that escapes the provider layer and is raised again
//! by `agent.run_single` / `web_channel.run_chat_task`.
//!
//! **Exception: the OpenAI-compatible "unknown model" shape** (`Model 'X'
//! is not available. Use GET /openai/v1/models …`) is now emitted by the
//! OpenHuman hosted backend too, in response to user-configured model ids
//! that aren't in the backend's registry. Pinned by
//! [`is_openai_compatible_unknown_model_message`]. The HTTP-layer wrapper
//! drops the polarity guard for that specific shape so the same body is
//! treated as user-state regardless of provider — see TAURI-RUST-2Z1
//! where a user-typed `MiniMax-M2.7-highspeed` model id (plus two
//! `custom:` fallback variants from their own `model_fallbacks` config)
//! was rejected with this wire shape and otherwise reached Sentry.
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
        // OPENHUMAN-TAURI-R1 — custom_openai upstream 403 with body
        // `{"error":{"message":"This model is not available in your region.","code":403}}`.
        // User picked a model the provider blocks for their account's
        // region. Sentry has no remediation; user must switch model.
        "not available in your region",
        // OPENHUMAN-TAURI-R4 — Doubao / ChatGLM-style model allowlist
        // enforcement. Body: `{"reason":"ModelNotAllowed",...}`. Match
        // lowercased — the provider sends the camelCase token as a
        // sentinel `reason` value.
        "modelnotallowed",
        // OPENHUMAN-TAURI-YC — user-supplied custom_openai API key was
        // rejected by upstream with the OpenAI-compatible
        // `{"error":{"type":"invalid_authentication_error",...}}`
        // envelope. Anchored on the type token (stable across providers
        // that emit this OpenAI-compatible body).
        "invalid_authentication_error",
        // OPENHUMAN-TAURI-S5 — OpenRouter 402 when the user is out of
        // credits. Body always carries "requires more credits, or fewer
        // max_tokens"; pin to the unique-enough credits phrase. (The
        // separate `billing_error` classifier handles our own
        // OpenHuman-backend balance gate; this catches the third-party
        // OpenRouter shape that re-emits via `agent.run_single`.)
        "requires more credits",
        // OPENHUMAN-TAURI-Y0 — litellm-style proxy rejected the model
        // id pre-routing with `Invalid model name passed in model=…`.
        // Anchored on the `passed in model=` suffix so a stray "invalid
        // model name" log line elsewhere does not classify.
        "invalid model name passed in model=",
        // OPENHUMAN-TAURI-JN / -KB — custom provider proxy that fronts
        // multiple upstream APIs surfaces a "you haven't configured the
        // upstream provider yet" 401/404 as `{"error":{"message":"No
        // active credentials for provider: openai",...}}`. The
        // remediation is "add the upstream API key in Settings".
        "no active credentials for provider",
        // OPENHUMAN-TAURI-JK — litellm github_copilot proxy 400 driven
        // by the user's missing / expired Copilot OAuth scope. The body
        // always starts with the `litellm.BadRequestError:` envelope.
        // Anchor to that prefix-shaped substring so we don't catch
        // unrelated 400s that merely mention litellm in passing.
        "litellm.badrequesterror",
        // OPENHUMAN-TAURI-J2 / -J5 / -J4 — litellm-compatible
        // envelope with `"type":"not_found_error"` carrying "model 'X'
        // not found". Distinct from the existing `model_not_found`
        // phrase: that's the `code` field used by OpenAI-native bodies;
        // this is the `type` field used by litellm/Anthropic-style
        // envelopes for the same class of user-state error.
        "not_found_error",
        // TAURI-RUST-4NM — nvidia-nim (and compatible providers) return
        // `{"error":{"message":"model field is required","code":"missing_required_field"}}`
        // when the request body contains an empty `"model":""` field.
        // This is deterministic user-configuration state: the user's
        // provider string had no model id and the config entry has no
        // default_model. Factory now bails early, but guard the Sentry
        // signal for in-flight requests from older configs.
        // Note: match on the message phrase only — "missing_required_field"
        // is too generic and would incorrectly suppress unrelated errors.
        "model field is required",
        // TAURI-RUST-2G (~2684 events) / TAURI-RUST-2F (~950 events) —
        // thinking-mode model (DeepSeek-R1 / Moonshot K2-thinking on
        // `provider=cloud` custom_openai) rejects a follow-up turn that
        // doesn't echo the prior assistant's `reasoning_content` field.
        // Body shape (backtick-quoted JSON literal in the upstream body):
        // `{"error":{"message":"The `reasoning_content` in the thinking
        // mode must be passed back to the API.",...}}`. The
        // provider-contract gap is on our side, but until the thinking-
        // mode round-tripping ships in the inference layer, every affected
        // turn fires a fresh Sentry event — and the UI already surfaces
        // the actionable error to the user. Anchor on the unique
        // `thinking mode must be passed back` substring so the match
        // doesn't depend on the upstream's backtick-quoting around
        // `reasoning_content` (some provider versions ship without them).
        "thinking mode must be passed back",
        // TAURI-RUST-4XK (~649 events) — Ollama Cloud subscription gate.
        // Body: `{"error":"this model requires a subscription, upgrade for
        // access: https://ollama.com/upgrade (ref: <uuid>)"}` on a 403
        // Forbidden from `compatible::OpenAiCompatibleProvider` with
        // `name = "ollama"`. User-state: the model picked in Settings is
        // a paid-tier Ollama Cloud model the user's account doesn't
        // cover. The UI surfaces an actionable upgrade link in the
        // remediation message itself.
        "requires a subscription, upgrade for access",
    ];

    let lower = body.to_ascii_lowercase();
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

/// Returns true if a provider error body matches the OpenAI-compatible
/// "unknown model" shape — anchored on the `/openai/v1/models`
/// remediation hint the upstream returns alongside `Model 'X' is not
/// available.`.
///
/// This is a strict subset of [`is_provider_config_rejection_message`]:
/// the same phrase already lives in that predicate's list. The narrower
/// helper exists so the HTTP-layer wrapper
/// ([`super::ops::is_provider_config_rejection_http`]) can drop its
/// `provider != openhuman_backend::PROVIDER_LABEL` polarity guard for
/// this specific body shape — the OpenHuman hosted backend now emits the
/// same OpenAI-compatible "Model 'X' is not available" wire body in
/// response to user-configured unknown model ids, so the original
/// polarity assumption ("only third-party providers speak this dialect")
/// no longer holds.
///
/// Drops TAURI-RUST-2Z1 (per-attempt) — the aggregate sibling
/// TAURI-RUST-2Z2 is already covered by the message-only classifier in
/// [`crate::core::observability::expected_error_kind`].
pub fn is_openai_compatible_unknown_model_message(body: &str) -> bool {
    body.to_ascii_lowercase().contains("/openai/v1/models")
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
    fn detects_wave4_sentry_bodies() {
        // Real wire bodies pulled from the OPENHUMAN-TAURI-* Sentry
        // events the Wave 4 phrases drop.
        for (sentry_id, body) in [
            (
                "R1",
                r#"custom_openai API error (403 Forbidden): {"error":{"message":"This model is not available in your region.","code":403}}"#,
            ),
            (
                "R4",
                r#"custom_openai API error (403 Forbidden): {"code":403,"reason":"ModelNotAllowed","message":"模型不允许访问","metadata":{"request_id":"2026051706431574423265420620337"}}"#,
            ),
            (
                "YC",
                r#"custom_openai API error (401 Unauthorized): {"error":{"message":"Invalid Authentication","type":"invalid_authentication_error"}}"#,
            ),
            (
                "S5",
                r#"custom_openai API error (402 Payment Required): {"error":{"message":"This request requires more credits, or fewer max_tokens. You requested up to 65536 tokens, but can only afford 597.","type":"insufficient_credits"}}"#,
            ),
            (
                "Y0",
                r#"custom_openai API error (400 Bad Request): {"error":{"message":"{'error': '/chat/completions: Invalid model name passed in model=reasoning-v1. Call `/v1/models` to view available models for your key.'}","type":"None"}}"#,
            ),
            (
                "JN",
                r#"custom_openai Responses API error: {"error":{"message":"No active credentials for provider: openai","type":"invalid_request_error","code":"model_not_found"}}"#,
            ),
            (
                "KB",
                r#"OpenHuman API error (404 Not Found): {"error":{"message":"No active credentials for provider: openai","type":"invalid_request_error","code":"model_not_found"}}"#,
            ),
            (
                "JK",
                r#"custom_openai API error (400 Bad Request): {"error":{"message":"litellm.BadRequestError: Github_copilotException - Bad Request. Received Model Group=github_copilot/claude-haiku-4.5\nAvailable Model Group Fallbacks=None","type":null}}"#,
            ),
            (
                "J2",
                r#"custom_openai Responses API error: {"error":{"message":"model 'llama3.3' not found","type":"not_found_error","param":null,"code":null}}"#,
            ),
            (
                "J5",
                r#"custom_openai API error (404 Not Found): {"error":{"message":"model 'llama3.3' not found","type":"not_found_error","param":null,"code":null}}"#,
            ),
            (
                "J4",
                r#"custom_openai streaming API error (404 Not Found): {"error":{"message":"model 'llama3.3' not found","type":"not_found_error","param":null,"code":null}}"#,
            ),
            // TAURI-RUST-4NM — nvidia-nim (and compatible providers) return
            // this body when the request body has an empty `"model":""`.
            // This is user-configuration state: the provider string had no
            // model id and the config entry has no default_model set.
            (
                "4NM",
                r#"nvidia-nim API error (400 Bad Request): {"error":{"message":"model field is required","type":"invalid_request_error","param":null,"code":"missing_required_field"}}"#,
            ),
            (
                "TAURI-RUST-4XK",
                r#"ollama API error (403 Forbidden): {"error":"this model requires a subscription, upgrade for access: https://ollama.com/upgrade (ref: bc48f3c8-fba1-40b6-93a9-786a167d16f9)"}"#,
            ),
            (
                "TAURI-RUST-2G",
                r#"cloud API error (400 Bad Request): {"error":{"message":"The `reasoning_content` in the thinking mode must be passed back to the API.","type":"invalid_request_error","param":null,"code":"invalid_request_error"}}"#,
            ),
            (
                "TAURI-RUST-2F",
                r#"cloud streaming API error (400 Bad Request): {"error":{"message":"The `reasoning_content` in the thinking mode must be passed back to the API.","type":"invalid_request_error","param":null,"code":"invalid_request_error"}}"#,
            ),
        ] {
            assert!(
                is_provider_config_rejection_message(body),
                "OPENHUMAN-TAURI-{sentry_id} body must classify as provider config-rejection: {body:?}"
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

    #[test]
    fn unknown_model_helper_matches_openai_compatible_bodies() {
        // TAURI-RUST-2Z1 — the OpenHuman hosted backend now emits the
        // OpenAI-compatible "Model 'X' is not available" wire body for
        // user-configured unknown model ids. The helper is anchored on
        // the `/openai/v1/models` remediation hint so the same body shape
        // matches whether it came from a third-party `custom_openai`
        // upstream or our own backend.
        for body in [
            r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Model 'MiniMax-M2.7-highspeed' is not available. Use GET /openai/v1/models to list available models."}"#,
            r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Model 'custom:MiniMax-M2.7' is not available. Use GET /openai/v1/models to list available models."}"#,
            "Model 'deepseek-v4-pro' is not available. Use GET /openai/v1/models to list available models.",
        ] {
            assert!(
                is_openai_compatible_unknown_model_message(body),
                "TAURI-RUST-2Z1 body must classify as openai-compatible unknown model: {body:?}"
            );
            // Sanity: must remain a member of the broader phrase set so
            // the message-only classifier in
            // `crate::core::observability::expected_error_kind` keeps
            // demoting the aggregate (TAURI-RUST-2Z2).
            assert!(
                is_provider_config_rejection_message(body),
                "broader classifier must continue to match: {body:?}"
            );
        }
    }

    #[test]
    fn unknown_model_helper_rejects_other_config_rejection_phrases() {
        // Polarity exception must stay narrow: other config-rejection
        // shapes (DeepSeek `supported api model names are`, Moonshot
        // `invalid temperature`, OpenRouter `requires more credits`, …)
        // must still go through the provider-polarity guard so a
        // hypothetical regression where our own backend emits one of
        // those phrases reaches Sentry.
        for body in [
            "The supported API model names are deepseek-v4-pro or deepseek-v4-flash, but you passed reasoning-v1.",
            "invalid temperature: only 1 is allowed for this model",
            "The model `gpt-5.5` does not exist or you do not have access to it.",
            r#"{"error":{"message":"model not found","code":"model_not_found"}}"#,
            "This request requires more credits, or fewer max_tokens.",
        ] {
            assert!(
                !is_openai_compatible_unknown_model_message(body),
                "{body:?} must NOT match the narrow openai-compatible-unknown-model helper"
            );
        }
    }
}
