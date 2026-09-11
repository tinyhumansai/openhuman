use super::*;

#[test]
fn both_timeout_shapes_still_render_the_same_user_facing_copy() {
    // The telemetry split must not change what the user sees: either way the
    // turn ran out of time and the graceful `turn_timeout` copy is correct.
    let marker = super::super::web_errors::turn_timeout_error_message(600);
    let harness =
        "run timed out: tool call for run `agent_turn` exceeded its remaining wall-clock budget (26375 ms)";
    assert_eq!(classify_inference_error(&marker).error_type, "turn_timeout");
    assert_eq!(classify_inference_error(harness).error_type, "turn_timeout");
}

#[test]
fn classify_inference_error_rate_limited_surfaces_retry_after_seconds() {
    let raw = "openrouter API error (429 Too Many Requests): Retry-After: 30";
    let ClassifiedError {
        error_type: category,
        message,
        ..
    } = classify_inference_error(raw);
    assert_eq!(category, "rate_limited");
    assert!(
        message.contains("Try again in 30 seconds"),
        "must surface the parsed retry-after window: {message}"
    );
    assert!(
        message.contains("retry in this thread"),
        "must clarify the thread isn't blocked: {message}"
    );
}

#[test]
fn classify_inference_error_rate_limited_no_retry_after_omits_hint() {
    let raw = "openrouter API error (429 Too Many Requests)";
    let ClassifiedError {
        error_type: category,
        message,
        ..
    } = classify_inference_error(raw);
    assert_eq!(category, "rate_limited");
    // Generic copy must still describe the situation accurately.
    assert!(message.contains("transient upstream limit"));
    // No hallucinated countdown when none was parsed.
    assert!(
        !message.contains("Try again in"),
        "must NOT invent a retry-after when none was parsed: {message}"
    );
}

#[test]
fn classify_inference_error_rate_limited_handles_fractional_and_minute_windows() {
    // Fractional seconds round up — never tell the user to retry
    // sooner than the upstream actually allows.
    let message = classify_inference_error("429 Too Many Requests: retry_after: 2.4").message;
    assert!(
        message.contains("Try again in 3 seconds"),
        "fractional 2.4 must round up to 3: {message}"
    );

    // Long windows switch to a "minutes" rendering at the 90s
    // threshold so the user gets a less precise but more readable
    // hint.
    let message = classify_inference_error("429 Too Many Requests: Retry-After: 180").message;
    assert!(
        message.contains("about 3 minutes"),
        "180s must render as minutes: {message}"
    );
}

#[test]
fn classify_inference_error_rate_limited_minute_window_uses_singular_and_rounds_up() {
    // CodeRabbit on #2371: the 90–119s band used to render
    // "about 1 minutes" (floor + missing plural handling). Round
    // up + singular/plural now produces "about 2 minutes" for 90s
    // (since 90s ceils to 2 minutes) and "about 2 minutes" for
    // 119s (ditto). 60s lands in the seconds band; 61s is the
    // smallest minute-band input but still <90 so seconds; 90s is
    // the first true minute-band input.
    let m_90 = classify_inference_error("429 Too Many Requests: Retry-After: 90").message;
    assert!(
        m_90.contains("about 2 minutes"),
        "90s must round up to 2 minutes (not floor to 1): {m_90}"
    );
    let m_119 = classify_inference_error("429 Too Many Requests: Retry-After: 119").message;
    assert!(
        m_119.contains("about 2 minutes"),
        "119s must round up to 2 minutes: {m_119}"
    );
    // Exactly 60-multiple inputs above the 90s threshold render as
    // exact minutes with no round-up bump.
    let m_120 = classify_inference_error("429 Too Many Requests: Retry-After: 120").message;
    assert!(
        m_120.contains("about 2 minutes"),
        "exact 120s must stay as 2 minutes: {m_120}"
    );
}

#[test]
fn classify_inference_error_rate_limited_parses_quoted_json_retry_after() {
    // CodeRabbit on #2371: a serialised provider body like
    // {"retry_after": 30} would previously miss every prefix
    // because the quote stopped `lower.find("retry_after:")` from
    // matching. The parser now strips quotes so the JSON-key shape
    // resolves the same as the unquoted header shape.
    let ClassifiedError {
        error_type: category,
        message,
        ..
    } = classify_inference_error(
        r#"openrouter API error (429 Too Many Requests): {"retry_after": 30, "code": "rate_limited"}"#,
    );
    assert_eq!(category, "rate_limited");
    assert!(
        message.contains("Try again in 30 seconds"),
        "quoted JSON retry_after must be parsed: {message}"
    );
}

// ── Structured rate-limit metadata (issue #2606) ──────────────
//
// The classifier MUST surface the structured fields the frontend
// needs to render a countdown / retry / fallback UI without having
// to regex the message string:
//   - retry_after_ms — raw, milliseconds, machine-readable
//   - source         — "provider" | "openhuman_budget" | "agent_loop"
//   - provider       — name extracted from upstream string when present
//   - retryable      — same-thread retry safe? (false for non-retryable 429)
//   - fallback_available — Some(false) once the reliable provider has
//                          exhausted its model_fallbacks chain
//
// These supplement the existing `error_type` token and `message`
// text — they do NOT replace either, and pre-#2371 consumers that
// read only the tuple shape keep working.

#[test]
fn classify_inference_error_rate_limited_returns_structured_retry_after_ms() {
    let raw = "openrouter API error (429 Too Many Requests): Retry-After: 30";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "rate_limited");
    assert_eq!(
        classified.retry_after_ms,
        Some(30_000),
        "30s Retry-After must surface as 30000ms on the structured payload \
         so the FE can render a countdown without regexing the message: \
         got {:?}",
        classified.retry_after_ms
    );
    assert_eq!(
        classified.source, "provider",
        "upstream 429 must classify source=provider, not openhuman_budget"
    );
    assert_eq!(
        classified.retryable, true,
        "transient upstream 429 must allow same-thread retry"
    );
    assert_eq!(
        classified.provider.as_deref(),
        Some("openrouter"),
        "provider name must be extracted from the '<provider> API error' \
         prefix: got {:?}",
        classified.provider
    );
}

#[tokio::test]
async fn start_chat_chat_error_event_serializes_structured_fields_to_json_wire() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    // The JSON-RPC SSE endpoint emits chat_error by running
    // `serde_json::to_value(&event)` over the WebChannelEvent struct
    // (see `core/socketio.rs::emit_web_channel_event`). This pins the
    // resulting JSON keys so the frontend contract stays stable: the
    // FE reads exactly `error_source`, `error_retryable`,
    // `error_retry_after_ms`, `error_provider`, `error_fallback_available`
    // off the SSE payload — the same keys our Rust struct serializes
    // to with `#[serde(rename_all = "snake_case")]`.
    //
    // Also asserts the additive contract: when these fields are None
    // they MUST be omitted from the JSON (older FE clients that don't
    // know about them keep working) — `skip_serializing_if =
    // "Option::is_none"` on every new field carries this guarantee.
    set_test_forced_run_chat_task_error(Some(
        "openrouter API error (429 Too Many Requests): Retry-After: 7",
    ))
    .await;

    let mut rx = subscribe_web_channel_events();
    let request_id = start_chat(
        "wire-shape-client",
        "wire-shape-thread",
        "Please summarize this in one line.",
        None,
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("start_chat should accept valid request");

    let recv = timeout(Duration::from_secs(20), async move {
        loop {
            let event = rx.recv().await.expect("event stream should stay open");
            if event.event != "chat_error" {
                continue;
            }
            if event.request_id != request_id {
                continue;
            }
            return event;
        }
    })
    .await
    .expect("expected chat_error event for started chat request");

    let json = serde_json::to_value(&recv).expect("WebChannelEvent must serialize");
    assert_eq!(
        json.get("error_type").and_then(|v| v.as_str()),
        Some("rate_limited")
    );
    assert_eq!(
        json.get("error_source").and_then(|v| v.as_str()),
        Some("provider")
    );
    assert_eq!(
        json.get("error_retryable").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        json.get("error_retry_after_ms").and_then(|v| v.as_u64()),
        Some(7_000)
    );
    assert_eq!(
        json.get("error_provider").and_then(|v| v.as_str()),
        Some("openrouter")
    );
    // No fallback signal in this error string → field omitted.
    assert!(
        json.get("error_fallback_available").is_none(),
        "None fields MUST be omitted from JSON for additive wire compat: {json}"
    );

    // Pin the additive contract: serializing a default (no error)
    // event must NOT introduce any of the new keys.
    let empty = crate::core::socketio::WebChannelEvent {
        event: "chat_done".to_string(),
        ..Default::default()
    };
    let empty_json = serde_json::to_value(&empty).expect("Default WebChannelEvent serializes");
    for key in [
        "error_source",
        "error_retryable",
        "error_retry_after_ms",
        "error_provider",
        "error_fallback_available",
    ] {
        assert!(
            empty_json.get(key).is_none(),
            "{key} must be omitted when None so older FE clients aren't surprised: {empty_json}"
        );
    }

    set_test_forced_run_chat_task_error(None).await;
}

#[tokio::test]
async fn start_chat_emits_structured_rate_limit_metadata_on_chat_error_event() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    // End-to-end wire check for issue #2606: when run_chat_task fails
    // with a 429-shaped error string, the published `chat_error`
    // WebChannelEvent on the bus MUST carry the structured fields the
    // FE needs (retry_after_ms, source, provider, retryable) — not
    // just the message text. This is the contract older PR #2371
    // landed in *message* form but kept off the wire as structured
    // metadata.
    set_test_forced_run_chat_task_error(Some(
        "openrouter API error (429 Too Many Requests): Retry-After: 30",
    ))
    .await;

    let mut rx = subscribe_web_channel_events();
    let request_id = start_chat(
        "rate-limit-client",
        "rate-limit-thread",
        "Please summarize this in one line.",
        None,
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("start_chat should accept valid request");

    let recv = timeout(Duration::from_secs(20), async move {
        loop {
            let event = rx.recv().await.expect("event stream should stay open");
            if event.event != "chat_error" {
                continue;
            }
            if event.request_id != request_id {
                continue;
            }
            return event;
        }
    })
    .await
    .expect("expected chat_error event for started chat request");

    assert_eq!(
        recv.error_type.as_deref(),
        Some("rate_limited"),
        "error_type token unchanged for backward compat"
    );
    assert_eq!(
        recv.error_source.as_deref(),
        Some("provider"),
        "upstream 429 must classify as provider source on the wire: {recv:?}"
    );
    assert_eq!(
        recv.error_retryable,
        Some(true),
        "transient upstream 429 must be retryable on the wire: {recv:?}"
    );
    assert_eq!(
        recv.error_retry_after_ms,
        Some(30_000),
        "30s Retry-After must surface as 30000ms on the wire (FE countdown): \
         {recv:?}"
    );
    assert_eq!(
        recv.error_provider.as_deref(),
        Some("openrouter"),
        "provider name must reach the wire so the FE can show \"openrouter is throttling\": \
         {recv:?}"
    );

    set_test_forced_run_chat_task_error(None).await;
}

#[test]
fn classify_inference_error_action_budget_marks_source_openhuman_not_provider() {
    // OpenHuman's SecurityPolicy per-hour cap is NOT a provider 429 —
    // it's a local safety cap. The structured payload must reflect
    // that so the FE doesn't tell the user to switch providers, and
    // doesn't promise a fallback (the cap applies to OpenHuman itself).
    let raw = "Rate limit exceeded: action budget exhausted";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "action_budget_exceeded");
    assert_eq!(
        classified.source, "openhuman_budget",
        "OpenHuman's own per-hour cap must NOT be tagged as a provider source"
    );
    assert!(
        classified.retryable,
        "the budget decays gradually so the same thread CAN retry"
    );
    assert_eq!(
        classified.retry_after_ms, None,
        "the SecurityPolicy decay isn't expressed as a discrete Retry-After"
    );
    assert_eq!(
        classified.provider, None,
        "no upstream provider is implicated — the limit is local"
    );
}

#[test]
fn classify_inference_error_max_iterations_marks_source_agent_loop_retryable() {
    let raw = "Agent exceeded maximum tool iterations (12)";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "max_iterations");
    assert_eq!(
        classified.source, "agent_loop",
        "the agent's own iteration cap must be its own source category"
    );
    assert!(
        classified.retryable,
        "user CAN re-ask in the same thread once the underlying limit clears"
    );
    assert_eq!(classified.retry_after_ms, None);
}

#[test]
fn classify_inference_error_non_retryable_429_business_quota_unsets_retry() {
    // Business 429s (plan doesn't include the model, balance exhausted,
    // known business codes like Z.AI 1311/1113) are 429s in shape but
    // retrying is futile until the user changes plan/billing. The FE
    // should hide the "Retry" button when retryable=false.
    let cases: &[&str] = &[
        "openrouter API error (429): plan does not include this model",
        "openai API error (429): insufficient_balance",
        "zai API error (429): code 1311 quota exhausted",
        "zai API error (429): error code 1113",
    ];
    for raw in cases {
        let classified = classify_inference_error(raw);
        assert_eq!(
            classified.error_type, "rate_limited",
            "still a 429 by classification: {raw}"
        );
        assert!(
            !classified.retryable,
            "business-quota 429 must NOT be marked retryable: {raw}"
        );
    }
}

#[test]
fn classify_inference_error_retryable_429_keeps_retry_flag() {
    // Vanilla transient 429 — retry is the right answer. The FE should
    // show the countdown + retry button.
    let raw = "openai API error (429 Too Many Requests): Retry-After: 5";
    let classified = classify_inference_error(raw);
    assert!(
        classified.retryable,
        "vanilla transient 429 must remain retryable: {classified:?}"
    );
    assert_eq!(classified.retry_after_ms, Some(5_000));
    assert_eq!(classified.source, "provider");
}

#[test]
fn classify_inference_error_extracts_provider_name_lowercase() {
    // The "<provider> API error" envelope from
    // inference::provider::ops::api_error is the canonical source we
    // pull the provider name from. Lowercased so the wire value is
    // stable across providers' own capitalisation.
    let cases: &[(&str, Option<&str>)] = &[
        (
            "openrouter API error (429): rate limited",
            Some("openrouter"),
        ),
        (
            "Anthropic API error (429): too many requests",
            Some("anthropic"),
        ),
        ("custom_openai API error (429): {}", Some("custom_openai")),
        // No "API error" infix — no extraction (the upstream didn't
        // come through the standard wrapper).
        ("connect timed out after 30s", None),
    ];
    for (raw, want) in cases {
        let classified = classify_inference_error(raw);
        assert_eq!(
            classified.provider.as_deref(),
            *want,
            "provider extraction mismatch for {raw:?}"
        );
    }
}

#[test]
fn classify_inference_error_fallback_chain_exhausted_sets_false() {
    // Once reliable.rs's `format_failure_aggregate` has emitted "All
    // providers/models failed", we KNOW no fallback remains. The FE
    // must not offer a "Try fallback" CTA in that case.
    let raw = "openrouter API error (429): rate limited\nAll providers/models failed. Attempts:\nprovider=openai model=gpt-4 attempt 1/3: rate_limited";
    let classified = classify_inference_error(raw);
    assert_eq!(
        classified.fallback_available,
        Some(false),
        "aggregate marker must surface as fallback_available=Some(false): {classified:?}"
    );
}

#[test]
fn classify_inference_error_no_fallback_signal_means_unknown() {
    // Single-provider 429 with no aggregate marker — the classifier
    // cannot tell whether a fallback exists. Must surface None
    // ("unknown") so the FE doesn't promise something we can't deliver.
    let raw = "openrouter API error (429): rate limited";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.fallback_available, None);
}

#[test]
fn classify_inference_error_auth_marks_non_retryable_config_source() {
    let raw = "openai API error (401 Unauthorized): invalid api key";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "auth_error");
    assert_eq!(classified.source, "config");
    assert!(
        !classified.retryable,
        "401 won't recover until the user updates settings"
    );
}

#[test]
fn classify_inference_error_billing_402_distinguished_from_provider_429() {
    // Acceptance criteria for #2606: distinguish upstream provider
    // throttling (429) from OpenHuman budget/billing limits (402).
    let raw = "OpenHuman API error (402 Payment Required): top up to continue";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "budget_exhausted");
    assert_eq!(
        classified.source, "openhuman_billing",
        "402 must NOT share source with provider 429"
    );
    assert!(!classified.retryable);
}

#[test]
fn classify_inference_error_upstream_provider_402_is_not_openhuman_billing() {
    // Regression for the inverse of the #2606 acceptance criterion: a
    // 402 carrying an upstream provider envelope must be attributed to
    // that provider, NOT to OpenHuman's own billing surface. Tagging it
    // openhuman_billing misled the FE into pointing the user at OpenHuman
    // credits when in fact their provider plan / balance is the issue.
    let cases: &[&str] = &[
        "openrouter API error (402 Payment Required): insufficient balance",
        "openai API error (402): payment required",
    ];
    for raw in cases {
        let classified = classify_inference_error(raw);
        assert_eq!(
            classified.error_type, "budget_exhausted",
            "still budget_exhausted by classification: {raw}"
        );
        assert_eq!(
            classified.source, "provider",
            "upstream provider 402 must be sourced to the provider, not OpenHuman billing: {raw}"
        );
        assert!(!classified.retryable);
    }
}

#[test]
fn classify_inference_error_non_retryable_429_message_routes_to_settings() {
    // Companion to the non-retryable 429 branch: when retry is futile
    // (plan limit, insufficient balance, business code), the message
    // MUST NOT tell the user "you can retry in this thread" — that's
    // the transient-429 copy. The non-retryable copy points to billing
    // / plan / model settings instead.
    let cases: &[&str] = &[
        "openrouter API error (429): plan does not include this model",
        "openai API error (429): insufficient_balance",
    ];
    for raw in cases {
        let classified = classify_inference_error(raw);
        assert!(
            !classified.retryable,
            "guard against accidental retryability flip: {raw}"
        );
        assert!(
            !classified.message.contains("retry in this thread"),
            "non-retryable 429 copy MUST NOT promise same-thread retry: {}",
            classified.message
        );
        assert!(
            classified.message.contains("Settings")
                || classified.message.contains("plan")
                || classified.message.contains("credits"),
            "non-retryable 429 copy must route to billing/plan/settings: {}",
            classified.message
        );
    }
}

#[test]
fn classify_inference_error_retryable_429_message_keeps_retry_hint() {
    // Companion: a vanilla transient 429 must still surface the
    // "retry in this thread" reassurance — the message branch must
    // not bleed across.
    let raw = "openrouter API error (429 Too Many Requests): Retry-After: 5";
    let classified = classify_inference_error(raw);
    assert!(classified.retryable);
    assert!(
        classified.message.contains("retry in this thread"),
        "transient 429 must still reassure same-thread retry: {}",
        classified.message
    );
    assert!(
        classified.message.contains("Try again in 5 seconds"),
        "transient 429 must surface the retry-after hint: {}",
        classified.message
    );
}

#[test]
fn generic_error_copy_is_sanitized_and_has_discord_report_action() {
    let message = generic_inference_error_user_message();
    assert!(message.contains("Something went wrong. Please try again."));
    assert!(message.contains("This error has been reported."));
    assert!(message.contains(
        "<openhuman-link path=\"community/discord-report\">Report on Discord</openhuman-link>"
    ));
}

#[test]
fn classify_inference_error_empty_response_is_actionable_and_retryable() {
    // #3092 / #3119: the dominant chat-error cause (Sentry TAURI-RUST-4JW,
    // 986+ events). An empty provider completion must get an actionable,
    // retryable message — NOT the generic "Something went wrong" dead-end.
    let raw = "run_chat_task failed client_id=abc thread_id=t-1 request_id=r-1 \
               error=The model returned an empty response. Please try again.";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "empty_response");
    assert_ne!(
        classified.error_type, "inference",
        "empty response must NOT fall through to the generic catch-all"
    );
    assert!(
        classified.retryable,
        "empty response is transiently retryable"
    );
    assert_eq!(classified.source, "agent_loop");
    assert!(
        classified.message.contains("Connections → API keys → LLM"),
        "must give the actionable model-switch remedy: {}",
        classified.message
    );
    assert!(
        !classified.message.contains("Something went wrong"),
        "must not be the generic apology: {}",
        classified.message
    );
}

#[test]
fn codex_oauth_expiry_classifies_as_provider_error_not_session_expired() {
    // The error from store.rs contains "authentication token is expired" (an
    // is_openai_oauth_session_expired_message marker) wrapped in the factory
    // chain prefix. Verify it routes to `provider_error` (Codex reconnect),
    // NOT `session_expired` (OpenHuman sign-in), so the user gets the correct
    // remedy. (#5869)
    let err = "[chat-factory] openai oauth lookup failed: \
               Codex authentication token is expired — refresh failed: \
               Token exchange failed: HTTP 401 Unauthorized: \
               {\"error\":{\"message\":\"Could not validate your token. \
               Please try signing in again.\",\"code\":\"token_expired\"}}. \
               Please reconnect Codex in Settings \u{2192} Integrations.";
    let classified = classify_inference_error(err);
    assert_eq!(
        classified.error_type, "provider_error",
        "Codex OAuth expiry must not route to the OpenHuman sign-in flow"
    );
    assert!(
        classified.message.contains("Integrations"),
        "must surface the Settings → Integrations remedy: {}",
        classified.message
    );
    assert!(
        !classified
            .message
            .contains("Your OpenHuman session has expired"),
        "must not show the app-session copy: {}",
        classified.message
    );
}

#[test]
fn non_codex_token_expired_does_not_classify_as_codex_oauth() {
    // A generic provider error containing "token_expired" (without the Codex
    // sentinel prefix) must NOT be classified as a Codex OAuth expiry. Without
    // the "codex authentication token is expired" guard, the old broad predicate
    // would have matched. (#5869)
    let err = "openai error 401: {\"error\":{\"code\":\"token_expired\",\
               \"message\":\"Please try signing in again.\"}}";
    let classified = classify_inference_error(err);
    assert_ne!(
        classified.provider.as_deref(),
        Some("openai_codex"),
        "generic token_expired error must not route to the Codex reconnect flow: {}",
        classified.message
    );
}
