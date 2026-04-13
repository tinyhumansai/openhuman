//! Build the turn, dispatch `agent.run_turn`, parse the reply.
//!
//! This is the core of the triage pipeline. It:
//!
//! 1. Resolves a provider via [`super::routing::resolve_provider`]
//!    (commit 1 = always remote; commit 2 = local-or-remote with probe
//!    + cache).
//! 2. Looks up the `trigger_triage` built-in agent definition from
//!    the global [`AgentDefinitionRegistry`].
//! 3. Builds a [`ChatMessage`] history: the definition's system
//!    prompt body + a user message summarising the envelope.
//! 4. Dispatches the turn through the existing
//!    [`crate::openhuman::agent::bus::AGENT_RUN_TURN_METHOD`] native
//!    request so tests can override the handler via
//!    [`crate::openhuman::agent::bus::mock_agent_run_turn`].
//! 5. Parses the reply with
//!    [`super::decision::parse_triage_decision`] — tolerant enough to
//!    accept whatever 1B-parameter output looks like today.
//!
//! ## Why `run_tool_call_loop` doesn't care about `tools_registry = []`
//!
//! The triage agent has `named = []` in its TOML (zero tools). The
//! `run_tool_call_loop` implementation in
//! `src/openhuman/agent/harness/tool_loop.rs` handles an empty registry
//! by just doing a plain `chat_with_history` under the hood — no tool
//! schemas are sent to the backend. That's exactly what we want: one
//! LLM round-trip, no chained tool calls.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context};

use crate::core::event_bus::{request_native_global, NativeRequestError};
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::harness::definition::{AgentDefinition, PromptSource};
use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::config::MultimodalConfig;
use crate::openhuman::providers::ChatMessage;

use crate::openhuman::config::Config;

use super::decision::{parse_triage_decision, ParseError, TriageDecision};
use super::envelope::TriggerEnvelope;
use super::events;
use super::routing::{self, resolve_provider_with_config, ResolvedProvider};

/// Agent definition id for the built-in triage classifier. Hard-coded
/// so a rogue workspace TOML can't override it to point at a different
/// agent — the triage pipeline is tightly coupled to the prompt
/// contract in `trigger_triage/prompt.md`.
pub const TRIGGER_TRIAGE_AGENT_ID: &str = "trigger_triage";

/// How much of the raw payload we inline into the user message. Picked
/// so a huge Gmail body cannot blow the local model's context window.
/// The classifier only needs the gist — downstream agents (orchestrator,
/// trigger_reactor) can re-read the full payload if they need it.
const PAYLOAD_INLINE_LIMIT_BYTES: usize = 8 * 1024;

/// Final output of a single triage run — the parsed decision plus
/// bookkeeping fields published on the domain event.
#[derive(Debug, Clone)]
pub struct TriageRun {
    pub decision: TriageDecision,
    pub used_local: bool,
    pub latency_ms: u64,
}

/// Run the triage classifier against a trigger envelope.
///
/// This is the main entry point for trigger classification. It performs the following:
/// 1. Resolves an appropriate provider (preferring local LLMs for speed).
/// 2. Dispatches a single LLM turn using the `trigger_triage` archetype.
/// 3. Parses the resulting JSON decision.
/// 4. If the local attempt fails or produces garbage, automatically retries on a
///    remote provider for maximum reliability.
///
/// On success returns a [`TriageRun`] containing the decision and performance metrics.
pub async fn run_triage(envelope: &TriggerEnvelope) -> anyhow::Result<TriageRun> {
    // Load the config once and reuse it for both the first attempt and
    // any retry that falls back to remote. `Config::load_or_init` is
    // relatively heavy (disk + env merge) so paying it twice would
    // double the tail latency of a degraded-local trigger.
    let config = Config::load_or_init()
        .await
        .context("loading config for triage turn")?;
    let resolved = resolve_provider_with_config(&config)
        .await
        .context("resolving provider for triage turn")?;

    // First attempt. On success, publish latency + return.
    match run_triage_with_resolved(resolved, envelope).await {
        Ok(run) => Ok(run),
        Err(first_err)
            if first_err
                .downcast_ref::<TurnOutcomeFailure>()
                .is_some_and(|f| f.used_local) =>
        {
            // Local turn failed — mark cache degraded, rebuild a remote
            // provider from the SAME config, and retry once. If that
            // also fails, publish `TriggerEscalationFailed` and return.
            tracing::warn!(
                error = %first_err,
                "[triage::evaluator] local attempt failed — retrying on remote"
            );
            routing::mark_degraded().await;
            let remote = resolve_provider_with_config(&config)
                .await
                .context("rebuilding remote provider for triage retry")?;
            debug_assert!(!remote.used_local, "mark_degraded must force remote");
            match run_triage_with_resolved(remote, envelope).await {
                Ok(run) => {
                    tracing::info!(
                        label = %envelope.display_label,
                        "[triage::evaluator] remote retry succeeded after local failure"
                    );
                    Ok(run)
                }
                Err(second_err) => {
                    let reason =
                        format!("local then remote both failed: {first_err} / {second_err}");
                    events::publish_failed(envelope, &reason);
                    Err(anyhow!(reason))
                }
            }
        }
        Err(err) => {
            // Remote attempt failed, or local attempt failed in a way
            // that isn't eligible for retry (e.g. registry missing
            // built-in — rebuilding won't help). Publish failure.
            let reason = format!("{err}");
            events::publish_failed(envelope, &reason);
            Err(err)
        }
    }
}

/// Sentinel error wrapper the retry path looks for. We attach it to
/// recoverable failures (handler error on local, parse failure on
/// local) so `run_triage` can decide whether a second attempt is worth
/// running. Unrecoverable failures (missing registry, missing built-in
/// definition, etc.) surface as plain `anyhow::Error` and skip the
/// retry loop.
#[derive(Debug)]
struct TurnOutcomeFailure {
    used_local: bool,
    kind: &'static str,
    message: String,
}

impl std::fmt::Display for TurnOutcomeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "triage {} ({}): {}",
            self.kind,
            if self.used_local { "local" } else { "remote" },
            self.message
        )
    }
}

impl std::error::Error for TurnOutcomeFailure {}

/// Inner half of [`run_triage`] that takes an already-resolved
/// [`ResolvedProvider`] instead of calling `routing::resolve_provider`.
///
/// Split out so unit tests can inject a stub provider without loading
/// real config from disk or constructing a real backend client. The
/// public [`run_triage`] path delegates here after resolving. Commit 2
/// will also call this from the "parse-failed → retry on remote" path
/// with a second resolved provider.
pub async fn run_triage_with_resolved(
    resolved: ResolvedProvider,
    envelope: &TriggerEnvelope,
) -> anyhow::Result<TriageRun> {
    let started = Instant::now();

    tracing::debug!(
        source = %envelope.source.slug(),
        label = %envelope.display_label,
        external_id = %envelope.external_id,
        provider = %resolved.provider_name,
        used_local = resolved.used_local,
        "[triage::evaluator] starting triage turn"
    );

    let ResolvedProvider {
        provider,
        provider_name,
        model,
        used_local,
    } = resolved;

    // ── Look up the built-in agent definition ──────────────────────
    let registry = AgentDefinitionRegistry::global().ok_or_else(|| {
        anyhow!(
            "AgentDefinitionRegistry not initialised — did startup wiring \
             skip `init_global`?"
        )
    })?;
    let definition = registry.get(TRIGGER_TRIAGE_AGENT_ID).ok_or_else(|| {
        anyhow!("built-in `{TRIGGER_TRIAGE_AGENT_ID}` definition missing from registry")
    })?;

    // ── Build the turn history ──────────────────────────────────────
    let system_prompt = extract_inline_prompt(definition).context(
        "trigger_triage agent definition must ship an inline prompt body \
         (bug: the built-in loader injects one at startup)",
    )?;
    let user_message = render_user_message(envelope);
    let history = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(&user_message),
    ];

    // ── Dispatch via the native bus so tests can stub it ────────────
    let request = AgentTurnRequest {
        provider: Arc::clone(&provider),
        history,
        tools_registry: Arc::new(Vec::new()),
        provider_name: provider_name.clone(),
        model: model.clone(),
        temperature: definition.temperature,
        silent: true,
        channel_name: "triage".to_string(),
        multimodal: MultimodalConfig::default(),
        // Single round-trip: the triage agent has zero tools so this
        // cap is only a safety net.
        max_tool_iterations: 1,
        on_delta: None,
        // The triage classifier runs against an empty tools registry
        // by design and emits a structured JSON decision rather than
        // calling tools — record the agent identity for tracing but
        // leave the visible-tool filter unset so the legacy unfiltered
        // behaviour is preserved.
        target_agent_id: Some("trigger_triage".to_string()),
        visible_tool_names: None,
        extra_tools: Vec::new(),
    };
    tracing::debug!(
        provider = %provider_name,
        model = %model,
        used_local = used_local,
        "[triage::evaluator] dispatching {AGENT_RUN_TURN_METHOD}"
    );
    let response = match request_native_global::<AgentTurnRequest, AgentTurnResponse>(
        AGENT_RUN_TURN_METHOD,
        request,
    )
    .await
    {
        Ok(r) => r,
        Err(err) => {
            let message = match &err {
                NativeRequestError::HandlerFailed { message, .. } => message.clone(),
                other => format!("[agent.run_turn dispatch] {other}"),
            };
            tracing::warn!(
                error = %message,
                used_local = used_local,
                "[triage::evaluator] agent turn dispatch failed"
            );
            // Wrap in TurnOutcomeFailure so the outer `run_triage` can
            // decide whether to retry on remote. Only local failures
            // are retry-eligible.
            return Err(anyhow!(TurnOutcomeFailure {
                used_local,
                kind: "handler",
                message,
            }));
        }
    };

    // ── Parse the classifier's reply ────────────────────────────────
    let decision = match parse_triage_decision(&response.text) {
        Ok(d) => d,
        Err(parse_err) => {
            tracing::warn!(
                error = %parse_err,
                reply_chars = response.text.chars().count(),
                used_local = used_local,
                "[triage::evaluator] classifier reply did not parse"
            );
            return Err(anyhow!(TurnOutcomeFailure {
                used_local,
                kind: "parser",
                message: format_parse_error(&parse_err),
            }));
        }
    };

    let latency_ms = started.elapsed().as_millis() as u64;
    tracing::info!(
        source = %envelope.source.slug(),
        label = %envelope.display_label,
        action = %decision.action.as_str(),
        used_local = used_local,
        latency_ms = latency_ms,
        "[triage::evaluator] classifier decision produced"
    );

    Ok(TriageRun {
        decision,
        used_local,
        latency_ms,
    })
}

/// Pull the inline prompt body out of the definition. Built-ins always
/// use [`PromptSource::Inline`] — the `BUILTINS` loader in
/// `agent/agents/mod.rs` injects the rendered `prompt.md` at startup.
/// Returning an option here (rather than panicking) lets the caller
/// surface a clean error to downstream logging.
fn extract_inline_prompt(def: &AgentDefinition) -> Option<String> {
    match &def.system_prompt {
        PromptSource::Inline(body) if !body.is_empty() => Some(body.clone()),
        _ => None,
    }
}

/// Render the user-side message that the classifier reads. The prompt
/// contract in `trigger_triage/prompt.md` describes the field layout;
/// we keep it trivially serialisable so a 1B model can reason over it.
fn render_user_message(envelope: &TriggerEnvelope) -> String {
    let payload_string = truncate_payload(&envelope.payload, PAYLOAD_INLINE_LIMIT_BYTES);
    format!(
        "SOURCE: {source}\n\
         DISPLAY_LABEL: {label}\n\
         EXTERNAL_ID: {eid}\n\
         PAYLOAD:\n{payload}",
        source = envelope.source.slug(),
        label = envelope.display_label,
        eid = envelope.external_id,
        payload = payload_string,
    )
}

/// Format a [`ParseError`] for inclusion in a [`TurnOutcomeFailure`]
/// message. Keeps the source-error wrapper out of the string so the
/// retry log stays readable.
fn format_parse_error(err: &ParseError) -> String {
    match err {
        ParseError::NoJsonObject => "classifier reply had no JSON object".to_string(),
        ParseError::InvalidJson(src) => format!("classifier JSON invalid: {src}"),
        ParseError::MissingTarget { action } => {
            format!("action `{action}` missing required target_agent/prompt")
        }
    }
}

/// Pretty-print `payload` as JSON, truncate if it exceeds `max_bytes`,
/// and leave a `[...truncated N bytes]` marker so the classifier
/// understands the input was abridged.
fn truncate_payload(payload: &serde_json::Value, max_bytes: usize) -> String {
    let pretty = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    if pretty.len() <= max_bytes {
        return pretty;
    }
    let dropped = pretty.len() - max_bytes;
    // Split at a char boundary at or below `max_bytes` so we never
    // produce an invalid UTF-8 slice when the payload contains
    // multi-byte characters.
    let mut end = max_bytes;
    while end > 0 && !pretty.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[...truncated {dropped} bytes]", &pretty[..end])
}

#[cfg(test)]
mod tests {
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
        def.system_prompt = PromptSource::Inline(builtin.prompt.to_string());
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
                    text: "Here's my call:\n```json\n{\"action\":\"drop\",\"reason\":\"test noise\"}\n```".to_string(),
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

        let envelope =
            TriggerEnvelope::from_composio("slack", "SLACK_MESSAGE", "t", "u", json!({}));

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
                        text: "{\"action\":\"acknowledge\",\"reason\":\"valid on retry\"}"
                            .to_string(),
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
}
