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
            "ollama API error: model 'bge-m3:latest' does not support chat — it appears to be an embedding or non-chat model. Assign a chat-capable model to this provider (e.g. in Connections → API keys → LLM), or pick a different model.",
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
         OpenHuman config, or change your default model in Connections → API keys → LLM.\n\n\
         All providers/models failed. Attempts:\n\
         provider=openhuman model=reasoning-quick-v1 attempt 1/3: non_retryable; \
         error=OpenHuman API error (401 Unauthorized): {\"success\":false,\"error\":\"Invalid token\"}",
        // 2) Unknown-model upstream cause.
        "The model `gpt-5.5` may not be available on your provider. \
         Configure a fallback chain via `reliability.model_fallbacks` in your \
         OpenHuman config, or change your default model in Connections → API keys → LLM.\n\n\
         All providers/models failed. Attempts:\n\
         provider=custom_openai model=gpt-5.5 attempt 1/3: non_retryable; \
         error=custom_openai API error (404 Not Found): {\"error\":\"model not found\"}",
        // 3) Region-block (R1-sibling) per-attempt cause.
        "The model `gpt-4o` may not be available on your provider. \
         Configure a fallback chain via `reliability.model_fallbacks` in your \
         OpenHuman config, or change your default model in Connections → API keys → LLM.\n\n\
         All providers/models failed. Attempts:\n\
         provider=custom_openai model=gpt-4o attempt 1/3: non_retryable; \
         error=custom_openai API error (403 Forbidden): {\"error\":{\"message\":\"This model is not available in your region.\"}}",
        // 4) Bare aggregate — minimal anchor surface.
        "The model `x` may not be available on your provider. \
         Configure a fallback chain via `reliability.model_fallbacks` in your \
         OpenHuman config, or change your default model in Connections → API keys → LLM.\n\n\
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
    // TAURI-RUST-GKV — the #2784 cloud-slug resolution guard catches the
    // empty-model state
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
