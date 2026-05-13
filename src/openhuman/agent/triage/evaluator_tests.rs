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
                reason.to_lowercase().contains("503") || reason.contains("cloud"),
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
