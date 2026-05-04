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
    assert!(out.len() <= 128 + 64); // generous upper bound for the marker
                                    // Round-trip to prove it's valid UTF-8 (otherwise format! would
                                    // have panicked — this assertion is belt-and-braces).
    let _ = out.as_str();
}

#[test]
fn extract_inline_prompt_returns_body_for_trigger_triage_builtin() {
    // Load the baked-in TOML+prompt directly so this test doesn't
    // depend on `AgentDefinitionRegistry::init_global` having been
    // called by the test runner.
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

// ── Bus dispatch integration test ───────────────────────────────
//
// Stubs `agent.run_turn` via `mock_agent_run_turn` and drives
// `run_triage_with_resolved` with an injected `ResolvedProvider`.
// Proves:
//   1. the evaluator routes its turn through the native bus
//   2. the dispatched `AgentTurnRequest` has the triage system
//      prompt, a user message carrying the envelope label, empty
//      tools_registry, and the `provider_name` / `model` the
//      resolver returned
//   3. a canned JSON reply is parsed into the correct
//      `TriageDecision`

/// Minimal `Provider` impl that satisfies the `Arc<dyn Provider>`
/// type in `ResolvedProvider`. The stubbed bus handler never
/// actually invokes any provider methods — if it did, these
/// methods would bail out loudly so the test fails fast.
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
        anyhow::bail!(
            "NoopProvider::chat_with_system should never be called — \
             the mock_agent_run_turn stub short-circuits before the \
             real handler hits any provider method"
        )
    }
}

fn fake_resolved(used_local: bool) -> ResolvedProvider {
    ResolvedProvider {
        provider: StdArc::new(NoopProvider) as StdArc<dyn Provider>,
        provider_name: "stub-provider".to_string(),
        model: "stub-model".to_string(),
        used_local,
    }
}

#[tokio::test]
async fn run_triage_dispatches_through_agent_run_turn_bus() {
    // Registry must be available before `run_triage_with_resolved`
    // looks up the `trigger_triage` definition. `init_global_builtins`
    // is a no-op on subsequent calls so parallel tests are safe.
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");

    let envelope = TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "trig-42",
        "uuid-42",
        json!({ "from": "ada@example.com", "subject": "ship it" }),
    );

    let stub_calls = StdArc::new(AtomicUsize::new(0));
    let stub_calls_handler = StdArc::clone(&stub_calls);

    // Capture of the dispatched request for deeper assertions
    // after the bus round-trip completes.
    let captured = StdArc::new(tokio::sync::Mutex::new(
        None::<(
            String, // provider_name
            String, // model
            usize,  // history length
            usize,  // tools_registry length
            String, // channel_name
            String, // system prompt body (first 200 chars)
            String, // user message body
        )>,
    ));
    let captured_handler = StdArc::clone(&captured);

    let _guard = mock_agent_run_turn(move |req| {
        let calls = StdArc::clone(&stub_calls_handler);
        let cap = StdArc::clone(&captured_handler);
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            let system_preview = req
                .history
                .first()
                .map(|m| m.content.chars().take(200).collect::<String>())
                .unwrap_or_default();
            let user_msg = req
                .history
                .get(1)
                .map(|m| m.content.clone())
                .unwrap_or_default();
            *cap.lock().await = Some((
                req.provider_name.clone(),
                req.model.clone(),
                req.history.len(),
                req.tools_registry.len(),
                req.channel_name.clone(),
                system_preview,
                user_msg,
            ));
            Ok(AgentTurnResponse {
                text:
                    "Here's my call:\n```json\n{\"action\":\"drop\",\"reason\":\"test noise\"}\n```"
                        .to_string(),
            })
        }
    })
    .await;

    let run = run_triage_with_resolved(fake_resolved(false), &envelope)
        .await
        .expect("run_triage should succeed with stub");

    // ── Stub was hit exactly once.
    assert_eq!(
        stub_calls.load(Ordering::SeqCst),
        1,
        "stub handler must be invoked exactly once per triage run"
    );

    // ── Dispatched request shape.
    let cap = captured.lock().await;
    let (provider_name, model, hist_len, tools_len, channel, sys_preview, user_msg) =
        cap.clone().expect("captured request");
    assert_eq!(provider_name, "stub-provider");
    assert_eq!(model, "stub-model");
    assert_eq!(hist_len, 2, "expected system + user message");
    assert_eq!(tools_len, 0, "trigger_triage has zero tools");
    assert_eq!(channel, "triage");
    assert!(
        sys_preview.to_lowercase().contains("trigger"),
        "system prompt should come from trigger_triage/prompt.md"
    );
    assert!(
        user_msg.contains("composio/gmail/GMAIL_NEW_GMAIL_MESSAGE"),
        "user message must carry the envelope display label, got: {user_msg}"
    );
    assert!(
        user_msg.contains("ada@example.com"),
        "user message must carry the payload, got: {user_msg}"
    );

    // ── Parsed decision matches the canned reply.
    assert_eq!(
        run.decision.action,
        crate::openhuman::agent::triage::TriageAction::Drop
    );
    assert_eq!(run.decision.reason, "test noise");
    assert!(!run.used_local);
}

#[tokio::test]
async fn remote_parse_failure_surfaces_as_error() {
    // When a remote turn (used_local=false) produces an unparseable
    // reply, the error surfaces directly — no retry is attempted
    // because there's no "better" provider to fall back to.
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");

    let envelope =
        TriggerEnvelope::from_composio("notion", "NOTION_PAGE_UPDATED", "t", "u", json!({}));

    let _guard = mock_agent_run_turn(move |_req| async move {
        Ok(AgentTurnResponse {
            text: "totally unparseable, no json here at all".to_string(),
        })
    })
    .await;

    let err = run_triage_with_resolved(fake_resolved(false), &envelope)
        .await
        .expect_err("remote parse failure must surface as error");
    let msg = err.to_string();
    assert!(
        msg.contains("parser") || msg.contains("JSON"),
        "expected parser error message, got: {msg}"
    );
}

#[tokio::test]
async fn local_parse_failure_is_retry_eligible() {
    // When a local turn (used_local=true) produces an unparseable
    // reply, the error is wrapped in TurnOutcomeFailure with
    // used_local=true, so the outer `run_triage` can detect it
    // and retry on remote.
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");

    let envelope = TriggerEnvelope::from_composio("slack", "SLACK_MESSAGE", "t", "u", json!({}));

    let _guard = mock_agent_run_turn(move |_req| async move {
        Ok(AgentTurnResponse {
            text: "no json here".to_string(),
        })
    })
    .await;

    let err = run_triage_with_resolved(fake_resolved(true), &envelope)
        .await
        .expect_err("local parse failure must surface as error");

    // The error must be a TurnOutcomeFailure with used_local=true
    // so the outer run_triage can detect it for retry.
    let failure = err
        .downcast_ref::<TurnOutcomeFailure>()
        .expect("error must be a TurnOutcomeFailure");
    assert!(
        failure.used_local,
        "TurnOutcomeFailure must report used_local=true for retry eligibility"
    );
    assert_eq!(failure.kind, "parser");
}

#[tokio::test]
async fn stateful_stub_simulates_local_garbage_then_remote_success() {
    // Proves the bus round-trip works with a stateful stub that
    // returns garbage on call 1 (simulating local) and valid JSON
    // on call 2 (simulating remote). This validates the exact
    // sequence the retry path in `run_triage` exercises.
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");

    let envelope = TriggerEnvelope::from_composio(
        "github",
        "GITHUB_PUSH",
        "t",
        "u",
        json!({ "ref": "refs/heads/main" }),
    );

    let call_counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&call_counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // First call: simulate garbage local response
                Ok(AgentTurnResponse {
                    text: "I have no idea what to do with this".to_string(),
                })
            } else {
                // Second call: valid JSON
                Ok(AgentTurnResponse {
                    text: "{\"action\":\"acknowledge\",\"reason\":\"valid on retry\"}".to_string(),
                })
            }
        }
    })
    .await;

    // Call 1: local (used_local=true) → expect parse failure
    let err = run_triage_with_resolved(fake_resolved(true), &envelope)
        .await
        .expect_err("first call should fail (garbage)");
    let failure = err.downcast_ref::<TurnOutcomeFailure>().unwrap();
    assert!(failure.used_local);

    // Call 2: remote (used_local=false) → expect success
    let run = run_triage_with_resolved(fake_resolved(false), &envelope)
        .await
        .expect("second call should succeed (valid JSON)");
    assert_eq!(
        run.decision.action,
        crate::openhuman::agent::triage::TriageAction::Acknowledge
    );
    assert_eq!(run.decision.reason, "valid on retry");
    assert!(!run.used_local);

    // Total: exactly 2 bus calls
    assert_eq!(call_counter.load(Ordering::SeqCst), 2);
}
