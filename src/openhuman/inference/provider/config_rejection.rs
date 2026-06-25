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
//! - `"Insufficient Balance"` (4ZF — DeepSeek custom BYO-key `402` when
//!   the user's DeepSeek account balance is exhausted)
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
        // TAURI-RUST-4ZF — DeepSeek (custom BYO-key) 402 when the user's
        // DeepSeek account balance is exhausted. Body carries the upstream
        // `{"error":{"message":"Insufficient Balance",...}}` envelope.
        // Same user-billing class as the OpenRouter S5 shape above.
        // NOTE: `is_budget_exhausted_message` (billing_error.rs) also
        // contains this phrase. In `expected_error_kind` (observability.rs)
        // this classifier is checked first (line 199 vs 205), so a re-
        // reported "Insufficient Balance" error routes to
        // `ProviderConfigRejection` rather than `BudgetExhausted`. Both
        // suppress Sentry at info-level — no event-volume regression — but
        // the telemetry `kind` tag becomes "provider_config_rejection".
        "insufficient balance",
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
        "model field is required",
        // TAURI-RUST-GKV (~2.3k events / 1 user) — the LOCAL form of the
        // 4NM empty-model state, caught one layer earlier. The #2784 guard
        // in `factory::make_cloud_provider_by_slug` bails BEFORE any
        // provider HTTP call when a `<slug>` provider string carries no
        // model and the `cloud_providers` entry has no `default_model`:
        //   "[chat-factory] no model configured: role '<r>' resolved to an
        //    empty model id for slug '<s>'. Include a model in the provider
        //    string (e.g. '<s>:<model-id>') or set default_model …".
        // User-state — the remediation is "add/pick a model in Settings →
        // LLM", which the user-facing copy surfaces; Sentry has no
        // remediation. Anchored on the role/slug-interpolation-free
        // substring, which is also `factory::NO_MODEL_CONFIGURED_ANCHOR`; a
        // round-trip test in `factory_tests.rs` couples the two so a
        // wording drift fails CI instead of silently re-flooding Sentry.
        "resolved to an empty model id",
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
        // TAURI-RUST-1V / OPENHUMAN-TAURI-4JS —
        // `reliable.rs::format_failure_aggregate` (no-configured-fallbacks
        // branch) wraps every exhausted `reliable_chat_with_system` turn
        // with:
        //
        //   "The model `<name>` may not be available on your provider.
        //    Configure a fallback chain via `reliability.model_fallbacks`
        //    in your OpenHuman config, or change your default model in
        //    Settings → AI.\n\nAll providers/models failed. Attempts:\n…"
        //
        // The aggregate fires once per turn regardless of the underlying
        // per-attempt cause (auth wall, unknown model, region block,
        // rate-limit cliff). All of those are user-actionable: pick a
        // different model, fix the credential, or configure fallbacks —
        // the message body literally tells the user how. Sentry has no
        // remediation path the per-attempt classifiers haven't already
        // covered at the lower layer (provider/ops.rs:486 publishes
        // SessionExpired, billing_error covers credit walls, etc.).
        //
        // Two anchors, both unique to this single emit site (verified via
        // grep across `src/`) and both present only in the no-configured-
        // fallbacks branch — the configured-fallbacks branch emits only
        // the bare "All providers/models failed. Attempts:\n…" dump, so
        // neither phrase fires on it (see the
        // `does_not_classify_reliable_aggregate_with_configured_fallbacks`
        // test). `may not be available on your provider` is the canonical
        // remediation-sentence phrase (TAURI-RUST-1V); the
        // `reliability.model_fallbacks` config path (OPENHUMAN-TAURI-4JS)
        // is kept as a redundant belt-and-braces anchor for the same line.
        "may not be available on your provider",
        "reliability.model_fallbacks",
        // TAURI-RUST-35 family — user picked a model that doesn't
        // implement tool calling, agent harness sent a tool spec
        // anyway, upstream rejected with `{"error":{"message":
        // "<model id> does not support tools",
        // "type":"invalid_request_error",...}}`. Same body across the
        // `cloud` / `ollama` / `custom_openai` provider prefixes — one
        // phrase drops all 10+ sibling Sentry issues currently
        // fragmented by model id (TAURI-RUST-35, -DF, -123, -4K7,
        // -4FS, -4F6, -2YA, -4KR, -4KH, -4KY — ~458 events). The user
        // must pick a tool-capable model; Sentry has no remediation.
        // NOTE: also pinned in the TAURI-RUST-4K7 capability-discovery
        // block above; both match the same phrase — the duplicate is
        // harmless (`.any()` short-circuits) and kept so each Sentry
        // family stays self-documenting.
        "does not support tools",
        // TAURI-RUST-ADC (~5.9k events / 10 users) — OpenRouter's
        // *router-level* phrasing of the same tool-capability user-state.
        // When the picked model has no provider endpoint that supports tool
        // calling, OpenRouter returns a 404 with `{"error":{"message":"No
        // endpoints found that support tool use. Try disabling \"<tool>\".
        // ..."}}`. The wording differs from the direct-provider `does not
        // support tools` body above ("tool use" vs "tools", prefixed with
        // "No endpoints found"), so it needs its own anchor. Same user-state
        // class: pick a tool-capable model. Surfaced most often by the
        // autonomous Subconscious loop, which additionally halts on this via
        // its own capability breaker (`subconscious/engine.rs`).
        "no endpoints found that support tool use",
        // TAURI-RUST-4P6 (~36.6k events / 2 users) — user picked an
        // *embedding* model (Ollama `bge-m3:latest`, OpenHuman's default
        // memory-tree embed model) as their chat model. Ollama rejects every
        // chat turn with `{"error":{"message":"\"bge-m3:latest\" does not
        // support chat","type":"invalid_request_error",...}}` on a 400. Same
        // user-state class as `does not support tools`: the model lacks the
        // chat capability and the user must pick a chat-capable model — Sentry
        // has no remediation. The 400 status bypasses the
        // `completion_only_404_guard` (404-only), so without this phrase the
        // raw body re-reports every turn. The companion `not_chat_capable_guard`
        // in `compatible.rs` rewrites the opaque upstream JSON into an
        // actionable "assign a chat-capable model" message that still carries
        // this substring, so it stays demoted.
        "does not support chat",
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
            // TAURI-RUST-ADC — OpenRouter router-level "no tool-use endpoint"
            // 404, surfaced by the autonomous Subconscious loop on a
            // content-safety model that supports no tools.
            (
                "ADC",
                r#"openrouter API error (404 Not Found): {"error":{"message":"No endpoints found that support tool use. Try disabling \"spawn_async_subagent\". To learn more about provider routing, visit: https://openrouter.ai/docs/guides/routing/provider-selection"}}"#,
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
            // TAURI-RUST-4P6 — user picked an embedding model
            // (`bge-m3:latest`) as their Ollama chat model. Ollama 400s every
            // chat turn. Verbatim wire body from Sentry issue 5338.
            (
                "TAURI-RUST-4P6",
                r#"ollama API error (400 Bad Request): {"error":{"message":"\"bge-m3:latest\" does not support chat","type":"invalid_request_error","param":null,"code":null}}"#,
            ),
            // Same shape after `not_chat_capable_guard` (compatible.rs)
            // rewrites it into the actionable message — must still classify so
            // the re-reported error stays demoted.
            (
                "TAURI-RUST-4P6-enriched",
                "ollama API error: model 'bge-m3:latest' does not support chat — it appears to be an embedding or non-chat model. Assign a chat-capable model to this provider (e.g. in Settings → AI), or pick a different model.",
            ),
        ] {
            assert!(
                is_provider_config_rejection_message(body),
                "OPENHUMAN-TAURI-{sentry_id} body must classify as provider config-rejection: {body:?}"
            );
        }
    }

    /// TAURI-RUST-4ZF — a user's custom BYO-key DeepSeek provider returns
    /// HTTP 402 with `{"error":{"message":"… Insufficient Balance …"}}`
    /// when their DeepSeek account is out of credits. Same user-billing
    /// class as the OpenRouter S5 "requires more credits" 402 already in
    /// the list — the remediation is "top up the provider account", which
    /// Sentry cannot act on. The DeepSeek wire token is `Insufficient
    /// Balance` (vs OpenRouter's `requires more credits`).
    #[test]
    fn detects_insufficient_balance_402_family() {
        for (sentry_id, body) in [
            // TAURI-RUST-4ZF — verbatim (truncated) from issue 5679,
            // model=`ds/deepseek-v4-flash`, provider=custom, status=402.
            (
                "4ZF",
                r#"custom API error (402 Payment Required): {"error":{"message":"[deepseek/deepseek-v4-flash] [402]: {\"error\":{\"message\":\"Insufficient Balance\",\"type\":\"unknown_error\",\"param\":null,\"code\":\"invali (reset after 57s)"}}"#,
            ),
            // Bare upstream envelope — what a future caller might re-emit
            // after unwrapping one layer.
            (
                "bare",
                r#"{"error":{"message":"Insufficient Balance","type":"unknown_error"}}"#,
            ),
        ] {
            assert!(
                is_provider_config_rejection_message(body),
                "TAURI-RUST-{sentry_id} insufficient-balance 402 must classify as provider config-rejection: {body:?}"
            );
        }
    }

    #[test]
    fn detects_reliable_aggregate_no_fallbacks_envelope() {
        // OPENHUMAN-TAURI-4JS — `reliable::format_failure_aggregate`
        // (no-configured-fallbacks branch) wraps every exhausted turn.
        // Pin a few realistic shapes:
        //
        //   1. Verbatim Sentry 4JS payload (auth wall as the per-attempt cause).
        //   2. Same aggregate, unknown-model upstream body (proves the matcher
        //      is per-emit-site, not per-underlying-cause).
        //   3. Same aggregate, region-block per-attempt body (R1-sibling cause).
        //   4. Bare two-line aggregate (only the literal prefix + an empty
        //      attempts dump).
        //
        // All four must classify; the unique anchor is the
        // `reliability.model_fallbacks` config path the message literally
        // tells the user to set.
        for raw in [
            // 1) Verbatim 4JS payload.
            "The model `reasoning-quick-v1` may not be available on your provider. \
             Configure a fallback chain via `reliability.model_fallbacks` in your \
             OpenHuman config, or change your default model in Settings → AI.\n\n\
             All providers/models failed. Attempts:\n\
             provider=openhuman model=reasoning-quick-v1 attempt 1/3: non_retryable; \
             error=OpenHuman API error (401 Unauthorized): {\"success\":false,\"error\":\"Invalid token\"}",
            // 2) Unknown-model upstream cause.
            "The model `gpt-5.5` may not be available on your provider. \
             Configure a fallback chain via `reliability.model_fallbacks` in your \
             OpenHuman config, or change your default model in Settings → AI.\n\n\
             All providers/models failed. Attempts:\n\
             provider=custom_openai model=gpt-5.5 attempt 1/3: non_retryable; \
             error=custom_openai API error (404 Not Found): {\"error\":\"model not found\"}",
            // 3) Region-block (R1-sibling) per-attempt cause.
            "The model `gpt-4o` may not be available on your provider. \
             Configure a fallback chain via `reliability.model_fallbacks` in your \
             OpenHuman config, or change your default model in Settings → AI.\n\n\
             All providers/models failed. Attempts:\n\
             provider=custom_openai model=gpt-4o attempt 1/3: non_retryable; \
             error=custom_openai API error (403 Forbidden): {\"error\":{\"message\":\"This model is not available in your region.\"}}",
            // 4) Bare aggregate — minimal anchor surface.
            "The model `x` may not be available on your provider. \
             Configure a fallback chain via `reliability.model_fallbacks` in your \
             OpenHuman config, or change your default model in Settings → AI.\n\n\
             All providers/models failed. Attempts:\n",
        ] {
            assert!(
                is_provider_config_rejection_message(raw),
                "OPENHUMAN-TAURI-4JS aggregate must classify as provider config-rejection: {raw:?}"
            );
        }
    }

    #[test]
    fn does_not_classify_reliable_aggregate_with_configured_fallbacks() {
        // The configured-fallbacks branch of `format_failure_aggregate`
        // emits ONLY the attempts dump (`"All providers/models failed.
        // Attempts:\n…"`), with no `reliability.model_fallbacks`
        // remediation hint — the user has already engaged with the knob,
        // so the aggregate is closer to a real diagnostic surface than a
        // user-config nudge. Without the anchor phrase, this matcher
        // must NOT fire on its own — only the per-attempt body
        // classifiers (#2786 SessionExpired, config_rejection siblings,
        // …) can demote it on a per-shape basis.
        let aggregate_with_fallbacks = "All providers/models failed. Attempts:\n\
             provider=openhuman model=gpt-5.5 attempt 1/3: non_retryable; \
             error=OpenHuman API error (404 Not Found): {\"error\":\"unknown model\"}";
        assert!(
            !is_provider_config_rejection_message(aggregate_with_fallbacks),
            "configured-fallbacks aggregate (no `reliability.model_fallbacks` anchor) \
             must NOT classify on the aggregate phrase alone"
        );
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
    fn detects_reliable_chain_exhaustion_rollup() {
        // TAURI-RUST-1V — `reliable.rs:325` rolls every attempt into
        // `All providers/models failed. Attempts:\n…\nThe model `<id>`
        // may not be available on your provider. Configure a fallback
        // chain via `reliability.model_fallbacks` in …`. The wrapped err
        // bubbles to `memory_sync::composio::bus` which previously
        // emitted it as a raw `tracing::error!` — 10.7k events / 14d on
        // self-hosted Sentry. The remediation lives entirely in the
        // user's `reliability.model_fallbacks` config; Sentry has no
        // remediation path.
        let rollup = "All providers/models failed. Attempts:\n\
            provider=openhuman model=gemini-3-flash-preview attempt 1/3: \
            non_retryable; error=custom_openai API error (404 Not Found): \
            <html>...</html>\n\
            The model `gemini-3-flash-preview` may not be available on \
            your provider. Configure a fallback chain via \
            `reliability.model_fallbacks` in your config to route around \
            unavailable models.";
        assert!(
            is_provider_config_rejection_message(rollup),
            "TAURI-RUST-1V multi-line rollup must classify as provider config-rejection"
        );

        // Single-line `reliable.rs:332` emission (without the outer
        // rollup wrapper) also matches — defensive against callers that
        // surface only the inner remediation message.
        let bare = "The model `chat-v1` may not be available on your provider. \
            Configure a fallback chain via `reliability.model_fallbacks` in …";
        assert!(
            is_provider_config_rejection_message(bare),
            "bare `may not be available on your provider` phrase must classify"
        );
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
    fn detects_nvidia_nim_missing_model_body() {
        // TAURI-RUST-4NM — nvidia-nim rejects requests with model="" with
        // `{"error":{"message":"model field is required",...}}`.
        let body = r#"nvidia-nim API error (400 Bad Request): {"error":{"message":"model field is required","type":"invalid_request_error","code":"missing_required_field"}}"#;
        assert!(
            is_provider_config_rejection_message(body),
            "TAURI-RUST-4NM body must classify as provider config-rejection: {body:?}"
        );
        // Also verify the bare phrase on its own (defense-in-depth path).
        assert!(is_provider_config_rejection_message(
            "model field is required"
        ));
    }

    #[test]
    fn detects_chat_factory_empty_model_local_bail() {
        // TAURI-RUST-GKV — the #2784 factory guard
        // (`make_cloud_provider_by_slug`) catches the empty-model state
        // BEFORE the provider HTTP call (the local form of 4NM) and bails
        // with this body (role/slug interpolated). Verbatim from Sentry
        // issue 18482 (role='chat', slug='nvidia').
        let body = "[chat-factory] no model configured: role 'chat' resolved to an empty model id \
                    for slug 'nvidia'. Include a model in the provider string (e.g. \
                    'nvidia:<model-id>') or set default_model on the cloud_providers entry for \
                    slug 'nvidia'.";
        assert!(
            is_provider_config_rejection_message(body),
            "TAURI-RUST-GKV empty-model bail must classify as provider config-rejection: {body:?}"
        );
        // Bare anchor on its own (the literal shared with
        // `factory::NO_MODEL_CONFIGURED_ANCHOR`).
        assert!(is_provider_config_rejection_message(
            "resolved to an empty model id"
        ));
        // Negative: a near-miss model-resolution error that does NOT carry
        // the anchor (or any other phrase) must stay Sentry-actionable.
        assert!(
            !is_provider_config_rejection_message(
                "could not resolve the model registry for slug 'nvidia'"
            ),
            "unrelated model-resolution error must not classify on the GKV anchor"
        );
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
