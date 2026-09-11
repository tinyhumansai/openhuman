use super::*;

#[test]
fn classify_inference_error_empty_response_copy_names_billing_remedy_and_drops_local_provider_misdirect(
) {
    // Issue #3335: the prior copy ("Try a different model or check your
    // local provider in Connections → API keys → LLM") sent Managed-route users
    // toward a remedy that does not exist for them. The common underlying
    // cause is credit exhaustion (issue #3386), so the revised copy must
    // name the credits / billing path explicitly, must NOT claim a "local
    // provider" exists, and must still offer the model-switch path for
    // users on self-hosted providers.
    let raw = "run_chat_task failed client_id=abc thread_id=t-1 request_id=r-1 \
               error=The model returned an empty response. Please try again.";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "empty_response");
    // New: names the credits / billing remedy (was absent in the old copy,
    // so Managed users had no way to self-diagnose credit exhaustion).
    assert!(
        classified.message.contains("Settings → Billing"),
        "must point at the billing surface for credit exhaustion: {}",
        classified.message
    );
    // New: drops the misleading "local provider" framing — the previous
    // copy made a false claim for Managed users where no local provider
    // exists.
    assert!(
        !classified.message.contains("local provider"),
        "must not claim a local provider exists: {}",
        classified.message
    );
    // Preserved: the model-switch remedy and the provider-config
    // settings deep link both still apply (some users hit empty response
    // because their custom OpenAI-compatible endpoint or local model is
    // misconfigured / unhealthy).
    assert!(
        classified.message.contains("different model"),
        "must keep the model-switch remedy: {}",
        classified.message
    );
    assert!(
        classified.message.contains("Connections → API keys → LLM"),
        "must keep the provider-config deep link: {}",
        classified.message
    );
    // Preserved: provider is intentionally None until the typed
    // `AgentError::EmptyProviderResponse` plumbs through a provider
    // identifier (see comment in `web_errors.rs::classify_inference_error`
    // empty_response arm).
    assert!(
        classified.provider.is_none(),
        "provider stays None until plumbed through the typed error: {:?}",
        classified.provider
    );
}

#[test]
fn classify_inference_error_vision_capability_is_non_retryable() {
    // A multimodal turn sent an image to a text-only model. Retrying the
    // same image+model can't help, so non-retryable with a switch-model hint.
    let raw = "provider_capability_error provider=web_channel capability=vision \
               message=received 1 image marker(s), but this provider does not support vision input";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "capability_unsupported");
    assert!(
        !classified.retryable,
        "same image + text-only model always fails"
    );
    assert!(
        classified.message.contains("vision-capable model"),
        "must point the user at a vision model: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_chat_template_rejection_is_not_blamed_on_the_model() {
    // #5291: LM Studio rendered the tool-loop message shape through Qwen 3's
    // chat template, which raised. Neither the model nor the temperature is at
    // fault, so neither may appear in the copy.
    let classified = classify_inference_error(LMSTUDIO_TEMPLATE_400);
    assert_eq!(classified.error_type, "chat_template_rejected");
    assert!(
        classified.message.contains("chat template"),
        "must name the template as the cause: {}",
        classified.message
    );
    // The two remediations that were previously offered for this body, both
    // dead ends: the config arm's "fix your model/routing" and the auth arm's
    // "check your API key".
    assert!(
        !classified.message.contains("Check your model and routing"),
        "must not send the user to model/routing settings: {}",
        classified.message
    );
    assert!(
        !classified.message.contains("check your API key"),
        "must not send the user to their API key: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_chat_template_wins_over_the_retry_aggregate() {
    // Both attempts fail and `format_failure_aggregate` wraps them. That
    // wrapper carries two phrases that other arms claim, and both misdiagnose
    // a template failure:
    //   - "Connections → API keys → LLM" contains the bare "api key" substring
    //     the auth_error arm matches on;
    //   - "may not be available on your provider" is a config-rejection phrase,
    //     rendered as "rejected the model or temperature setting" (#5291's
    //     reported symptom).
    // The template arm sits above both so neither can claim it.
    let aggregate = format!(
        "The model `qwen/qwen3.5-9b` may not be available on your provider. Configure a \
         fallback chain via `reliability.model_fallbacks` in your OpenHuman config, or change \
         your default model in Connections → API keys → LLM.\n\nAll providers/models failed. \
         Attempts:\nprovider=lmstudio model=qwen/qwen3.5-9b attempt 1/2: \
         {LMSTUDIO_TEMPLATE_400}\nprovider=lmstudio model=qwen/qwen3.5-9b attempt 2/2: \
         {LMSTUDIO_TEMPLATE_400}"
    );
    let classified = classify_inference_error(&aggregate);
    assert_eq!(classified.error_type, "chat_template_rejected");
    assert!(
        !classified.message.contains("model or temperature setting"),
        "the config-rejection copy must not win: {}",
        classified.message
    );
    assert!(
        !classified.message.contains("authentication issue"),
        "the auth copy must not win: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_temperature_rejection_still_classifies_as_config() {
    // Guard the other direction: the template arm sits directly above the
    // config-rejection arm and must not swallow a genuine model/temperature
    // rejection.
    let raw = "cloud API error (400): invalid temperature: only 1 is allowed for this model";
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "model_unavailable");
}

#[test]
fn classify_inference_error_generic_4xx_surfaces_provider_detail() {
    // A provider 400 none of the specific arms claimed: the real reason must
    // be quoted (via with_provider_detail) under a friendly, non-retryable
    // summary instead of the generic dead-end.
    let raw = r#"cloud API error (400 Bad Request): {"error":{"message":"tool_calls.id and tool_calls.type are required","type":"input_invalid"}}"#;
    let classified = classify_inference_error(raw);
    assert_eq!(classified.error_type, "provider_request_rejected");
    assert!(
        !classified.retryable,
        "4xx request rejection is not retryable"
    );
    assert!(
        classified.message.contains("Try a different model"),
        "friendly summary present: {}",
        classified.message
    );
    assert!(
        classified.message.contains("tool_calls.id"),
        "must quote the real provider reason: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_deepseek_reasoning_400_stays_config_rejection() {
    // ORDERING LOCK: the DeepSeek / Moonshot thinking-mode reasoning_content
    // round-trip 400 is ALREADY claimed by the provider-config-rejection arm
    // (the "thinking mode must be passed back" phrase, Sentry TAURI-RUST-2G /
    // -2F), which is ordered BEFORE the generic 4xx arm. So it must keep its
    // specific, actionable `model_unavailable` + Settings → LLM verdict and
    // NOT be downgraded to the generic provider_request_rejected copy. The
    // deeper round-trip fix (so the turn actually succeeds) is tracked in
    // #3197; this only asserts the user-facing classification stays specific.
    let raw = r#"cloud API error (400 Bad Request): {"error":{"message":"The reasoning_content in the thinking mode must be passed back","type":"invalid_request_error"}}"#;
    let classified = classify_inference_error(raw);
    assert_eq!(
        classified.error_type, "model_unavailable",
        "DeepSeek reasoning_content 400 must stay config-rejection, not generic 4xx"
    );
    assert_ne!(classified.error_type, "inference");
}

#[test]
fn classify_inference_error_invalid_temperature_400_stays_config_rejection() {
    // ORDERING LOCK: a 400 carrying the #2076 "invalid temperature" body must
    // keep its specific provider-config-rejection verdict (model_unavailable +
    // Settings → LLM remediation) and NOT be stolen by the generic 4xx arm,
    // which is ordered after it.
    let raw = r#"custom_openai API error (400 Bad Request): {"error":{"message":"invalid temperature: only 1 is allowed for this model","type":"invalid_request_error"}}"#;
    let classified = classify_inference_error(raw);
    assert_eq!(
        classified.error_type, "model_unavailable",
        "invalid-temperature 400 must stay config-rejection, not generic 4xx"
    );
    assert!(classified.message.contains("Settings → LLM"));
}

#[test]
fn classify_inference_error_model_not_found_404_stays_model_unavailable() {
    // ORDERING LOCK: a 404 "model does not exist" must keep its specific
    // model_unavailable verdict and NOT be stolen by the generic 4xx arm.
    let raw = r#"custom_openai API error (404 Not Found): {"error":{"message":"The model `gpt-5.5` does not exist or you do not have access to it.","code":"model_not_found"}}"#;
    let classified = classify_inference_error(raw);
    assert_eq!(
        classified.error_type, "model_unavailable",
        "model-not-found 404 must stay model_unavailable, not generic 4xx"
    );
}

#[test]
fn classify_inference_error_rate_limited_code_branches_first() {
    // F2: a managed RATE_LIMITED carries the structured `retryAfter`, which
    // the classifier must prefer and surface as a countdown hint.
    let raw = managed_error(
        "429 Too Many Requests",
        r#"{"error":{"message":"slow down","errorCode":"RATE_LIMITED","retryAfter":30}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(classified.error_type, "rate_limited");
    assert!(classified.retryable, "rate limit is retryable in-thread");
    assert_eq!(
        classified.retry_after_ms,
        Some(30_000),
        "structured retryAfter must drive retry_after_ms"
    );
    assert!(
        classified.message.contains("retry in this thread"),
        "must use the in-thread retry copy: {}",
        classified.message
    );
    assert!(
        classified.message.contains("30 seconds"),
        "must surface the retry countdown: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_user_insufficient_credits_is_the_only_top_up_case() {
    let raw = managed_error(
        "402 Payment Required",
        r#"{"error":{"errorCode":"USER_INSUFFICIENT_CREDITS","message":"no credits"}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(classified.error_type, "budget_exhausted");
    assert!(!classified.retryable, "out of credits is non-retryable");
    assert_eq!(classified.source, "openhuman_billing");
    assert!(
        classified.message.contains("out of credits")
            && classified.message.contains("Use Your Own Models"),
        "must offer top-up or BYO switch: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_upstream_unavailable_drops_user_blaming_copy() {
    // F4: operator fault → "temporarily unavailable — we've been notified",
    // never "check your API key".
    let raw = managed_error(
        "503 Service Unavailable",
        r#"{"error":{"errorCode":"UPSTREAM_UNAVAILABLE","message":"upstream 5xx"}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(classified.error_type, "provider_error");
    assert!(classified.retryable);
    assert!(
        classified.message.contains("temporarily unavailable")
            && classified.message.contains("we've been notified"),
        "must use the operator-fault copy: {}",
        classified.message
    );
    assert!(
        !classified.message.to_lowercase().contains("api key"),
        "must NOT blame the user's API key: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_model_unavailable_code_is_operator_fault_not_user_pick() {
    // F6: a managed MODEL_UNAVAILABLE is an operator registry/routing
    // misconfig — route to provider_error, NOT the user "pick a different
    // model" copy.
    let raw = managed_error(
        "404 Not Found",
        r#"{"error":{"errorCode":"MODEL_UNAVAILABLE","message":"no route for model"}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(
        classified.error_type, "provider_error",
        "managed MODEL_UNAVAILABLE is provider_error, not model_unavailable"
    );
    assert!(classified.retryable);
    assert!(
        classified.message.contains("temporarily unavailable"),
        "must use the operator-fault copy: {}",
        classified.message
    );
    assert!(
        !classified
            .message
            .to_lowercase()
            .contains("check your model"),
        "must NOT tell the user to pick a model: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_payload_too_large_is_new_non_retryable_bucket() {
    // F3.
    let raw = managed_error(
        "413 Payload Too Large",
        r#"{"error":{"errorCode":"PAYLOAD_TOO_LARGE","message":"too big"}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(classified.error_type, "payload_too_large");
    assert!(!classified.retryable, "payload too large is non-retryable");
    assert!(
        classified.message.contains("too large") && classified.message.contains("attachment"),
        "must use the shorten/remove-attachment copy: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_context_length_exceeded_reuses_context_overflow() {
    let raw = managed_error(
        "400 Bad Request",
        r#"{"error":{"errorCode":"CONTEXT_LENGTH_EXCEEDED","message":"too long"}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(classified.error_type, "context_overflow");
    assert!(!classified.retryable);
    assert!(
        classified.message.contains("start a new chat"),
        "must use the start-a-new-chat copy: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_user_param_bad_request_is_actionable() {
    let raw = managed_error(
        "400 Bad Request",
        r#"{"error":{"errorCode":"BAD_REQUEST","message":"unsupported parameter"}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(classified.error_type, "provider_request_rejected");
    assert!(!classified.retryable);
    assert!(
        classified.message.contains("Connections → API keys → LLM"),
        "user-param rejection points at Settings: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_malformed_bad_request_uses_rephrase_copy() {
    // F8: malformed (backend-flagged) → "rephrase, or new thread if it
    // persists" — NOT an outright "start a new thread".
    let raw = managed_error(
        "400 Bad Request",
        r#"{"error":{"errorCode":"BAD_REQUEST","malformed":true,"message":"unparseable"}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(classified.error_type, "provider_request_rejected");
    assert!(!classified.retryable);
    assert!(
        classified.message.contains("Try rephrasing it"),
        "malformed must use the rephrase copy: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_internal_error_is_generic_retryable() {
    let raw = managed_error(
        "500 Internal Server Error",
        r#"{"error":{"errorCode":"INTERNAL_ERROR","message":"boom"}}"#,
    );
    let classified = classify_inference_error(&raw);
    assert_eq!(classified.error_type, "inference");
    assert!(classified.retryable);
    assert!(
        classified.message.contains("we've been notified"),
        "must reassure the user it was reported: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_byo_no_code_keeps_user_actionable_copy() {
    // Managed-vs-BYO: a BYO provider key bad (direct 401, no errorCode) must
    // STILL get the user-actionable "check your API key" copy via the
    // substring fallback — the errorCode branch must not steal it.
    let auth = r#"openai API error (401 Unauthorized): {"error":{"message":"Incorrect API key provided"}}"#;
    let classified = classify_inference_error(auth);
    assert_eq!(classified.error_type, "auth_error");
    assert!(
        classified.message.contains("check your API key"),
        "BYO no-code 401 keeps the actionable copy: {}",
        classified.message
    );

    // BYO model misconfig (no errorCode) stays `model_unavailable` with the
    // "check your model settings" copy — distinct from the managed
    // MODEL_UNAVAILABLE provider_error route above (F6).
    let model = r#"custom_openai API error (404 Not Found): {"error":{"message":"model unavailable on this endpoint"}}"#;
    let classified = classify_inference_error(model);
    assert_eq!(classified.error_type, "model_unavailable");
    assert!(
        classified.message.contains("model settings"),
        "BYO no-code model error keeps the actionable copy: {}",
        classified.message
    );
}

#[test]
fn classify_inference_error_byo_with_error_code_token_is_not_managed() {
    // CodeRabbit: a BYO / direct-provider error whose body happens to carry an
    // `errorCode`-shaped field must NOT be classified on the managed-code
    // branch — the managed-envelope gate keeps it on the substring ladder so
    // the user-actionable BYO copy is preserved (and FE Sentry is unaffected).
    let raw = r#"custom_openai API error (429 Too Many Requests): {"error":{"errorCode":"RATE_LIMITED","message":"slow down"}}"#;
    let classified = classify_inference_error(raw);
    // Still classified as rate_limited via the substring ladder, but through
    // the BYO path: the message uses the existing substring-arm copy ("This is
    // a transient upstream limit"), NOT the managed errorCode copy ("You can
    // retry in this thread.").
    assert_eq!(classified.error_type, "rate_limited");
    assert!(
        classified.message.contains("transient upstream limit"),
        "BYO 429 must use the substring-arm copy, not the managed errorCode copy: {}",
        classified.message
    );
}

// ── Schema catalog ────────────────────────────────────────────

#[test]
fn web_channel_catalog_has_chat_and_cancel() {
    let s = all_web_channel_controller_schemas();
    let c = all_web_channel_registered_controllers();
    assert_eq!(s.len(), c.len());
    assert_eq!(s.len(), 4);
    let fns: Vec<&str> = s.iter().map(|x| x.function).collect();
    assert!(fns.contains(&"web_chat"));
    assert!(fns.contains(&"web_cancel"));
    assert!(fns.contains(&"web_queue_status"));
    assert!(fns.contains(&"web_queue_clear"));
}

#[test]
fn chat_schema_requires_client_thread_message() {
    let s = schemas("chat");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"client_id"));
    assert!(required.contains(&"thread_id"));
    assert!(required.contains(&"message"));
    // model_override and temperature must be optional.
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "model_override" && !f.required));
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "temperature" && !f.required));
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "profile_id" && !f.required));
}

#[test]
fn cancel_schema_requires_client_and_thread() {
    let s = schemas("cancel");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["client_id", "thread_id"]);
}

#[test]
fn unknown_schema_returns_unknown_fallback() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "channel");
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "error");
}

// ── Helpers ───────────────────────────────────────────────────

#[test]
fn key_for_is_thread_scoped_not_client_scoped() {
    // Runtime maps (THREAD_SESSIONS, IN_FLIGHT) key by thread_id ALONE, so the
    // key is stable across socket reconnects (which regenerate client_id).
    // Regression guard for the conversation-amnesia / dead-Cancel bug, where a
    // reconnect under a new client_id orphaned the thread's session + in-flight
    // handle.
    assert_eq!(key_for("thread-abc"), "thread-abc");
    assert_eq!(key_for(""), "");
    // The same thread resolves to the same key no matter which socket asks.
    assert_eq!(key_for("thread-xyz"), key_for("thread-xyz"));
}

#[test]
fn event_session_id_for_is_stable() {
    // Two calls with the same args must produce the same id.
    let a = event_session_id_for("c1", "t1");
    let b = event_session_id_for("c1", "t1");
    assert_eq!(a, b);
    // Different args → different id.
    let c = event_session_id_for("c2", "t1");
    assert_ne!(a, c);
}

#[test]
fn normalize_model_override_returns_none_for_empty_or_whitespace() {
    assert!(normalize_model_override(None).is_none());
    assert!(normalize_model_override(Some("".into())).is_none());
    assert!(normalize_model_override(Some("   ".into())).is_none());
}

#[test]
fn normalize_model_override_trims_value() {
    assert_eq!(
        normalize_model_override(Some("  gpt-4  ".into())),
        Some("gpt-4".to_string())
    );
}

// ── Broadcast events ──────────────────────────────────────────

#[test]
fn subscribe_web_channel_events_returns_receiver() {
    // Just confirm we can subscribe without panic.
    let _rx = subscribe_web_channel_events();
}

// ── Field builder helpers ─────────────────────────────────────

#[test]
fn required_string_marks_field_required() {
    let f = required_string("client_id", "c");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::String));
}

#[test]
fn optional_string_marks_field_optional() {
    let f = optional_string("model", "c");
    assert!(!f.required);
}

#[test]
fn optional_f64_marks_field_optional() {
    let f = optional_f64("temperature", "c");
    assert!(!f.required);
}

#[test]
fn json_output_is_required_json_field() {
    let f = json_output("ack", "c");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::Json));
}

#[test]
fn fingerprint_autonomy_change_is_cache_miss() {
    // Changing the agent-access policy must invalidate the cached agent so the
    // next turn rebuilds with the new SecurityPolicy (otherwise the tier change
    // silently does nothing — the bug this field fixes).
    let base = fp(None, None, "orchestrator", "anthropic:claude-sonnet-4-6");
    let mut changed = fp(None, None, "orchestrator", "anthropic:claude-sonnet-4-6");
    changed.autonomy_signature = "sig-after-tier-change".to_string();
    assert_ne!(
        base, changed,
        "a different autonomy signature must produce a cache miss"
    );
}

#[test]
fn fingerprint_model_registry_change_is_cache_miss() {
    // Toggling a model's "Supports vision" flag keeps the same model id, so it
    // changes neither model_override nor provider_binding. Without the registry
    // signature the stale Agent (old build-time model_vision) would be reused.
    let base = fp(None, None, "orchestrator", "openai:my-llava");
    let mut changed = fp(None, None, "orchestrator", "openai:my-llava");
    changed.model_registry_signature = "registry-after-vision-toggle".to_string();
    assert_ne!(
        base, changed,
        "a model_registry change (vision toggle) must produce a cache miss → rebuild"
    );
}

#[test]
fn fingerprint_profile_change_is_cache_miss() {
    // Switching the active agent profile on the same thread keeps the same
    // model/agent/provider, so without the profile signature the previous
    // profile's tool/skill/MCP/connector visibility would leak into the new
    // profile's turns. A different profile signature must force a rebuild.
    let base = fp(None, None, "orchestrator", "anthropic:claude-sonnet-4-6");
    let mut changed = fp(None, None, "orchestrator", "anthropic:claude-sonnet-4-6");
    changed.profile_signature = "profile-after-switch".to_string();
    assert_ne!(
        base, changed,
        "a different profile signature must produce a cache miss → rebuild"
    );
}

#[test]
fn fingerprint_identical_inputs_are_cache_hit() {
    let a = fp(None, None, "orchestrator", "anthropic:claude-sonnet-4-6");
    let b = fp(None, None, "orchestrator", "anthropic:claude-sonnet-4-6");
    assert_eq!(
        a, b,
        "identical fingerprints must compare equal (cache hit)"
    );
}
