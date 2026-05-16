use super::*;
use crate::openhuman::agent::agents::BUILTINS;
use crate::openhuman::agent::bus::{mock_agent_run_turn, AgentTurnResponse};
use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::providers::Provider;
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc as StdArc;

#[test]
fn render_user_message_includes_label_and_payload() {
    let env = TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "trig-1",
        "uuid-1",
        json!({ "from": "a@b.com", "subject": "hello" }),
    );
    let msg = render_user_message(&env);
    assert!(msg.contains("SOURCE: composio"));
    assert!(msg.contains("DISPLAY_LABEL: composio/gmail/GMAIL_NEW_GMAIL_MESSAGE"));
    assert!(msg.contains("EXTERNAL_ID: uuid-1"));
    assert!(msg.contains("a@b.com"));
}

#[test]
fn truncate_payload_marks_truncation_and_stays_valid_utf8() {
    let big = serde_json::Value::String("😀".repeat(10_000));
    let out = truncate_payload(&big, 128);
    assert!(out.contains("[...truncated"));
    assert!(out.len() <= 128 + 64);
    let _ = out.as_str();
}

#[test]
fn test_truncate_payload_utf8_boundary() {
    // Each "🦀" is 4 bytes. 10 of them = 40 bytes.
    let payload = json!({ "msg": "🦀".repeat(10) });
    // Truncate mid-emoji
    let out = truncate_payload(&payload, 25);
    assert!(out.contains("[...truncated"));
    // The part before truncation must be valid UTF-8
    let head = out.split('\n').next().unwrap();
    // Use a method that is stable on our toolchain
    assert!(!head.is_empty());
}

#[test]
fn extract_inline_prompt_returns_body_for_trigger_triage_builtin() {
    let builtin = BUILTINS
        .iter()
        .find(|b| b.id == TRIGGER_TRIAGE_AGENT_ID)
        .expect("trigger_triage built-in must be registered");
    let mut def: AgentDefinition = toml::from_str(builtin.toml).expect("TOML must parse");
    def.system_prompt = PromptSource::Dynamic(builtin.prompt_fn);
    let body = extract_inline_prompt(&def).expect("body should be present");
    assert!(
        body.to_lowercase().contains("trigger"),
        "prompt body should mention triggers"
    );
}

#[test]
fn classify_string_recognises_429_with_retry_after() {
    let err = classify_error("HTTP 429 Too Many Requests; Retry-After: 2".to_string());
    match err {
        ArmError::Retryable {
            retry_after_ms: Some(ms),
            ..
        } => {
            assert_eq!(ms, 2_000, "Retry-After: 2 → 2000 ms");
        }
        _ => panic!("expected Retryable with retry_after_ms"),
    }
}

#[test]
fn classify_string_recognises_5xx_as_transient() {
    let err = classify_error("upstream returned 503 Service Unavailable".to_string());
    assert!(
        matches!(err, ArmError::Retryable { .. }),
        "5xx should be Retryable"
    );
}

#[test]
fn classify_string_recognises_timeout_as_transient() {
    let err = classify_error("request timed out after 30s".to_string());
    assert!(
        matches!(err, ArmError::Retryable { .. }),
        "timeout should be Retryable"
    );
}

#[test]
fn classify_string_treats_auth_failure_as_fatal() {
    let err = classify_error("HTTP 401 unauthorized: invalid api key".to_string());
    assert!(
        matches!(err, ArmError::Fatal(_)),
        "auth failure should be Fatal"
    );
}

#[test]
fn classify_string_recognises_budget_exceeded_as_budget_exhausted() {
    // Matches the real payload that fired OPENHUMAN-TAURI-X in Sentry.
    let err = classify_error(
        "OpenHuman API error (400 Bad Request): {\"success\":false,\
         \"error\":\"Budget exceeded — add credits to continue\"}"
            .to_string(),
    );
    assert!(
        matches!(err, ArmError::BudgetExhausted(_)),
        "budget-exceeded must classify as BudgetExhausted, not Fatal (which pages Sentry)"
    );
}

#[test]
fn classify_string_recognises_budget_exceeds_your_limit() {
    // Exercises the "budget exceeds" needle added to NEEDLES as a
    // grammatically-correct variant of "budget exceeded" (past
    // tense vs. present tense) — matches e.g. "Your budget exceeds
    // your limit for this billing period."
    let err = classify_error("Your budget exceeds your limit for this billing period.".to_string());
    assert!(
        matches!(err, ArmError::BudgetExhausted(_)),
        "\"budget exceeds\" must classify as BudgetExhausted"
    );
}

#[test]
fn classify_string_does_not_match_budget_phrases_across_word_boundaries() {
    // Regression: a substring-based check would fire BudgetExhausted
    // on "stop updating" because the normalized text contains the
    // substring "top up" — across the boundary between "stop" and
    // "updating". Whole-word (token-window) matching prevents this.
    for msg in [
        "please stop updating the row",
        "stop updating now",
        "topup completed",
    ] {
        let err = classify_error(msg.to_string());
        assert!(
            matches!(err, ArmError::Fatal(_)),
            "expected Fatal (no spurious BudgetExhausted) for {msg:?}"
        );
    }
}

#[test]
fn classify_string_recognises_top_up_and_out_of_credits_as_budget_exhausted() {
    for msg in [
        "please top up your account",
        "you are out of credits, add credits to continue",
        "no remaining credits available",
    ] {
        let err = classify_error(msg.to_string());
        // Match by reference so `err` is only inspected (not moved) —
        // lets us reuse it for both the match-check and the failure
        // label without a double-move.
        let label = match &err {
            ArmError::Retryable { .. } => "Retryable",
            ArmError::Fatal(_) => "Fatal",
            ArmError::BudgetExhausted(_) => "BudgetExhausted",
            ArmError::SafetyFlagged(_) => "SafetyFlagged",
        };
        assert!(
            matches!(&err, ArmError::BudgetExhausted(_)),
            "expected BudgetExhausted for {msg:?}, got {label}"
        );
    }
}

fn arm_error_label(err: &ArmError) -> &'static str {
    match err {
        ArmError::Retryable { .. } => "Retryable",
        ArmError::Fatal(_) => "Fatal",
        ArmError::BudgetExhausted(_) => "BudgetExhausted",
        ArmError::SafetyFlagged(_) => "SafetyFlagged",
    }
}

#[test]
fn classify_string_recognises_prompt_guard_rejection_as_safety_flagged() {
    // OPENHUMAN-TAURI-X regression: the exact phrase our prompt-injection
    // guard emits from `agent::bus::enforce_prompt_input` /
    // `agent::harness::session::runtime` when it refuses to dispatch a
    // turn. Was previously classified as Fatal, which paged Sentry for
    // every adversarial Gmail message the triage agent saw.
    for raw in [
        "Prompt flagged for security review and was not processed.",
        "Prompt flagged for security review and was not processed. Please rephrase clearly.",
        "Prompt blocked by security policy.",
        // Wrapped in caller context (the agent bus surfaces the
        // verdict via `NativeRequestError::HandlerFailed` whose
        // message may carry additional prefix) — substring match
        // must still classify.
        "[agent.run_turn dispatch] HandlerFailed: Prompt flagged for security review and was not processed.",
    ] {
        let err = classify_error(raw.to_string());
        let label = arm_error_label(&err);
        assert!(
            matches!(&err, ArmError::SafetyFlagged(_)),
            "expected SafetyFlagged for {raw:?}, got {label}"
        );
    }
}

#[test]
fn classify_string_does_not_misclassify_unrelated_security_phrases() {
    // Conservative: only the verbatim guard phrases match. A doc-string
    // mentioning "security review" generically must NOT classify, or
    // we'd silently hide real issues from Sentry.
    for raw in [
        "scheduling a quarterly security review",
        "the runbook covers security policy violations",
        "ai security report attached",
    ] {
        let err = classify_error(raw.to_string());
        let label = arm_error_label(&err);
        assert!(
            matches!(&err, ArmError::Fatal(_)),
            "expected Fatal for {raw:?}, got {label}"
        );
    }
}

// ── Tiered fallback integration tests ───────────────────────────
//
// These drive `run_triage_with_arms` end-to-end through the agent
// bus, with a stateful stub that decides per-call whether to return
// success, a 429, a 5xx, or a fatal auth error. Each `cloud-then-
// local` test relies on call-ordering: cloud arm is exercised
// first; falling through to local arm uses a different
// `provider_name` we inspect to disambiguate.

struct NoopProvider;

#[async_trait]
impl Provider for NoopProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!("NoopProvider should never be called — bus mock short-circuits")
    }
}

fn cloud_arm() -> ResolvedProvider {
    ResolvedProvider {
        provider: StdArc::new(NoopProvider) as StdArc<dyn Provider>,
        provider_name: "stub-cloud".to_string(),
        model: "stub-cloud-model".to_string(),
        used_local: false,
    }
}

fn local_arm() -> ResolvedProvider {
    ResolvedProvider {
        provider: StdArc::new(NoopProvider) as StdArc<dyn Provider>,
        provider_name: "stub-local".to_string(),
        model: "stub-local-model".to_string(),
        used_local: true,
    }
}

fn envelope() -> TriggerEnvelope {
    TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "trig-x",
        "uuid-x",
        json!({ "from": "ada@example.com", "subject": "ship it" }),
    )
}

const VALID_JSON_REPLY: &str = "{\"action\":\"acknowledge\",\"reason\":\"all good\"}";

#[tokio::test]
async fn happy_path_returns_cloud_resolution() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");

    let _guard = mock_agent_run_turn(move |_req| async move {
        Ok(AgentTurnResponse {
            text: VALID_JSON_REPLY.to_string(),
        })
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("happy path must succeed");

    let run = outcome.into_decision().expect("decision");
    assert_eq!(run.resolution_path, TriageResolutionPath::Cloud);
    assert!(!run.used_local);
}

#[tokio::test]
async fn rate_limited_then_ok_marks_cloud_after_retry() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err("HTTP 429 Too Many Requests; Retry-After: 0".to_string())
            } else {
                Ok(AgentTurnResponse {
                    text: VALID_JSON_REPLY.to_string(),
                })
            }
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("retry path must succeed");

    let run = outcome.into_decision().expect("decision");
    assert_eq!(run.resolution_path, TriageResolutionPath::CloudAfterRetry);
    assert!(!run.used_local);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn double_429_falls_through_to_local_fallback() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                // Cloud calls #1 and #2 both 429.
                assert_eq!(req.provider_name, "stub-cloud", "first two calls hit cloud");
                Err("HTTP 429 Too Many Requests; Retry-After: 0".to_string())
            } else {
                // Third call should be the local arm.
                assert_eq!(req.provider_name, "stub-local", "fall-through hits local");
                Ok(AgentTurnResponse {
                    text: VALID_JSON_REPLY.to_string(),
                })
            }
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("local fallback must succeed");

    let run = outcome.into_decision().expect("decision");
    assert_eq!(run.resolution_path, TriageResolutionPath::LocalFallback);
    assert!(run.used_local);
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn cloud_5xx_falls_through_to_local_fallback() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                assert_eq!(req.provider_name, "stub-cloud");
                Err("upstream returned 502 Bad Gateway".to_string())
            } else {
                assert_eq!(req.provider_name, "stub-local");
                Ok(AgentTurnResponse {
                    text: VALID_JSON_REPLY.to_string(),
                })
            }
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("local fallback must succeed after 5xx");

    let run = outcome.into_decision().expect("decision");
    assert_eq!(run.resolution_path, TriageResolutionPath::LocalFallback);
}

#[tokio::test]
async fn cloud_then_local_failure_returns_deferred() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            // Every call fails transiently — cloud retry #1, retry #2, local.
            Err("HTTP 503 Service Unavailable".to_string())
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("Deferred is Ok, not Err");

    match outcome {
        TriageOutcome::Deferred {
            defer_until_ms,
            reason,
        } => {
            assert!(
                defer_until_ms > chrono::Utc::now().timestamp_millis(),
                "defer_until_ms must be in the future"
            );
            assert!(
                reason.contains("cloud retry exhausted"),
                "reason should reference the upstream failure: {reason}"
            );
        }
        TriageOutcome::Decision(_) => panic!("expected Deferred, got Decision"),
    }
    assert_eq!(counter.load(Ordering::SeqCst), 3, "1 + retry + local = 3");
}

#[tokio::test]
async fn fatal_cloud_error_short_circuits_without_local_attempt() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("HTTP 401 unauthorized: invalid api key".to_string())
        }
    })
    .await;

    let err = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect_err("auth failure must surface as Err");

    assert!(
        err.to_string().to_lowercase().contains("401")
            || err.to_string().to_lowercase().contains("unauthorized"),
        "expected auth-related error message, got: {err}"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "fatal cloud error should not retry or fall back"
    );
}

#[tokio::test]
async fn cloud_budget_exhausted_skips_retry_and_falls_to_local() {
    // Regression for OPENHUMAN-TAURI-X: when the cloud arm returns
    // "Budget exceeded — add credits to continue" we must not retry
    // the cloud arm (the second call would burn the same wall) and
    // we must not surface the error as Fatal (that paged Sentry).
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                assert_eq!(req.provider_name, "stub-cloud", "first call must hit cloud");
                Err("OpenHuman API error (400 Bad Request): \
                     {\"success\":false,\"error\":\"Budget exceeded — add credits to continue\"}"
                    .to_string())
            } else {
                // No second cloud call — should jump straight to local.
                assert_eq!(
                    req.provider_name, "stub-local",
                    "budget-exhausted must skip cloud retry and dispatch to local"
                );
                Ok(AgentTurnResponse {
                    text: VALID_JSON_REPLY.to_string(),
                })
            }
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("budget-exhausted must not surface as Err");

    let run = outcome.into_decision().expect("decision");
    assert_eq!(run.resolution_path, TriageResolutionPath::LocalFallback);
    assert!(run.used_local);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "1 cloud (rejected for budget) + 1 local = 2 (no cloud retry)"
    );
}

#[tokio::test]
async fn cloud_budget_exhausted_on_retry_falls_through_to_local() {
    // Variant of OPENHUMAN-TAURI-X: cloud arm trips a transient first
    // (so we *do* schedule the cloud retry), but the retry itself
    // comes back as Budget exceeded. We must not run a third cloud
    // call, and we must fall through to local rather than surface
    // the budget error as Fatal.
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            match n {
                0 => {
                    assert_eq!(req.provider_name, "stub-cloud", "first call must hit cloud");
                    Err("HTTP 503 Service Unavailable".to_string())
                }
                1 => {
                    assert_eq!(
                        req.provider_name, "stub-cloud",
                        "second call must be the cloud retry"
                    );
                    Err("OpenHuman API error (400 Bad Request): \
                         {\"success\":false,\"error\":\"Budget exceeded — add credits to continue\"}"
                        .to_string())
                }
                _ => {
                    // After the retry returned a budget error, we
                    // must jump straight to local — never a third
                    // cloud call.
                    assert_eq!(
                        req.provider_name, "stub-local",
                        "post-budget dispatch must land on local"
                    );
                    Ok(AgentTurnResponse {
                        text: VALID_JSON_REPLY.to_string(),
                    })
                }
            }
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("budget on retry must not surface as Err");

    let run = outcome.into_decision().expect("decision");
    assert_eq!(run.resolution_path, TriageResolutionPath::LocalFallback);
    assert!(run.used_local);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        3,
        "1 cloud (transient) + 1 cloud retry (budget) + 1 local = 3 (no extra cloud retry)"
    );
}

#[tokio::test]
async fn cloud_budget_exhausted_without_local_returns_deferred_not_err() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(
                "OpenHuman API error (400 Bad Request): Budget exceeded — add credits to continue"
                    .to_string(),
            )
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), None, &envelope())
        .await
        .expect("budget-exhausted with no local must be Deferred, not Err");

    match outcome {
        TriageOutcome::Deferred { reason, .. } => {
            assert!(
                reason.to_lowercase().contains("budget"),
                "deferral reason should name the budget cause: {reason}"
            );
        }
        TriageOutcome::Decision(_) => panic!("expected Deferred, got Decision"),
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "no retry — budget would block the second cloud call too"
    );
}

#[tokio::test]
async fn cloud_safety_flagged_then_local_flagged_defers_not_errs() {
    // Regression for OPENHUMAN-TAURI-X (regressed): the prompt-injection
    // guard fires the same verdict on cloud and local arms (the guard
    // runs in `agent::bus::run_turn` before either model is contacted),
    // so the realistic path is cloud-flagged → local-flagged → defer.
    // Previously this paged Sentry every time an adversarial Gmail
    // triage attempt fired (118 hits in 6 days).
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            // Verbatim string our prompt-injection guard emits.
            Err("Prompt flagged for security review and was not processed.".to_string())
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("safety-flagged must not surface as Err — must Defer");

    match outcome {
        TriageOutcome::Deferred { reason, .. } => {
            assert!(
                reason.to_lowercase().contains("prompt-guard"),
                "deferral reason should name the prompt-guard cause: {reason}"
            );
        }
        TriageOutcome::Decision(_) => panic!("expected Deferred, got Decision"),
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "1 cloud (safety-flagged, no retry) + 1 local (same verdict) = 2"
    );
}

#[tokio::test]
async fn cloud_safety_flagged_then_local_recovers_decides_on_local() {
    // Defense in depth: while in practice cloud + local share the
    // guard verdict, the chain must still cleanly dispatch to local
    // if cloud is the only arm that flagged. Locks in the
    // skip-retry-and-fall-through semantics independently of whether
    // local also flags.
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                assert_eq!(req.provider_name, "stub-cloud", "first call must hit cloud");
                Err("Prompt flagged for security review and was not processed.".to_string())
            } else {
                assert_eq!(
                    req.provider_name, "stub-local",
                    "safety-flagged on cloud must skip cloud retry and dispatch to local"
                );
                Ok(AgentTurnResponse {
                    text: VALID_JSON_REPLY.to_string(),
                })
            }
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("safety-flagged must not surface as Err");

    let run = outcome.into_decision().expect("decision");
    assert_eq!(run.resolution_path, TriageResolutionPath::LocalFallback);
    assert!(run.used_local);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "1 cloud (safety-flagged) + 1 local = 2 (no cloud retry)"
    );
}

#[tokio::test]
async fn cloud_safety_flagged_without_local_returns_deferred_not_err() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("Prompt flagged for security review and was not processed.".to_string())
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), None, &envelope())
        .await
        .expect("safety-flagged with no local must Defer, not Err");

    match outcome {
        TriageOutcome::Deferred { reason, .. } => {
            assert!(
                reason.to_lowercase().contains("prompt-guard"),
                "deferral reason should name the prompt-guard cause: {reason}"
            );
        }
        TriageOutcome::Decision(_) => panic!("expected Deferred"),
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "no retry — guard would block the second cloud call too"
    );
}

#[tokio::test]
async fn no_local_arm_returns_deferred_after_cloud_exhaustion() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("HTTP 503 Service Unavailable".to_string())
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), None, &envelope())
        .await
        .expect("Deferred is Ok");

    match outcome {
        TriageOutcome::Deferred { reason, .. } => {
            assert!(
                reason.contains("local arm unavailable"),
                "reason should explain the missing local arm: {reason}"
            );
        }
        TriageOutcome::Decision(_) => panic!("expected Deferred"),
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "1 cloud + 1 retry, no local"
    );
}
