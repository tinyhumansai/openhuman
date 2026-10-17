//! Hosted-turn integration coverage for the per-turn budget stop hook and the
//! turn-deadline wind-down, the two graceful pauses of a top-level turn.

use super::*;
use crate::agent::stop_hooks::BudgetStopHook;
use crate::agent::tinyagents::TurnModelSource;
use async_trait::async_trait;
use std::sync::Arc;
use tinyagents_harness::middleware::{StopDecision, StopHook, TurnState};
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyinference_llm::tool::ToolCall;
use tinyinference_llm::usage::Usage;
use tinytools::{Tool, ToolResult};

struct LimitedTool(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait]
impl Tool for LimitedTool {
    fn name(&self) -> &str {
        "limited_tool"
    }

    fn description(&self) -> &str {
        "test tool for budget stop coverage"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ToolResult::success("executed"))
    }
}

struct BudgetedToolModel(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait]
impl ChatModel<()> for BudgetedToolModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
        Some(PROFILE.get_or_init(|| {
            let mut profile = ModelProfile::default();
            profile.tool_calling = true;
            profile.model = Some("claude-sonnet-4-6".to_string());
            profile
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let call = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut response = ModelResponse::assistant("");
        response.usage = Some(Usage {
            input_tokens: 1_000_000,
            ..Usage::default()
        });
        if call == 0 {
            response.message.tool_calls = vec![ToolCall::new(
                "budgeted-call",
                "limited_tool",
                serde_json::json!({}),
            )];
            response.finish_reason = Some("tool_calls".to_string());
        } else {
            response = ModelResponse::assistant("done");
        }
        Ok(response)
    }
}

struct RecordingBudgetStopHook {
    inner: BudgetStopHook,
    reason: Arc<std::sync::Mutex<Option<String>>>,
}

#[async_trait]
impl StopHook<tinyagents_harness::cost::TurnCost> for RecordingBudgetStopHook {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn check(
        &self,
        state: &TurnState<'_, tinyagents_harness::cost::TurnCost>,
    ) -> StopDecision {
        let decision = self.inner.check(state).await;
        if let StopDecision::Stop { reason } = &decision {
            *self.reason.lock().expect("stop reason lock") = Some(reason.clone());
        }
        decision
    }
}

fn hosted_base() -> Arc<crate::agent::tinyagents::host::OpenHumanHostBase> {
    Arc::new(crate::agent::tinyagents::host::OpenHumanHostBase {
        config: Arc::new(crate::config::Config::default()),
        definitions: Arc::new(
            crate::agent::harness::host_definition::AgentDefinitionRegistry::builtins_only(),
        ),
        security_policy: Arc::new(crate::security::policy::SecurityPolicy::default()),
        post_turn_hooks: Vec::new(),
        session_definition: None,
    })
}

fn root_context(
    thread_id: &str,
    workspace: &str,
    progress: tokio::sync::mpsc::Sender<crate::agent::progress::AgentProgress>,
) -> crate::agent::tinyagents::host::OpenHumanRunContext {
    let mut context = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    context.origin = Some(crate::agent::turn_origin::AgentTurnOrigin::WebChat {
        thread_id: thread_id.to_string(),
        client_id: format!("client-{thread_id}"),
        request_id: Some(format!("request-{thread_id}")),
    });
    context.thread_id = Some(thread_id.to_string());
    context.workspace = Some(tinytools::WorkspaceDescriptor::new(workspace));
    context.progress = Some(progress);
    context
}

fn root_messages(label: &str) -> Vec<TranscriptMessage> {
    vec![
        TranscriptMessage::system(format!("system-{label}")),
        TranscriptMessage::user(format!("user-{label}")),
    ]
}

#[test]
fn budget_stop_hook_pauses_the_hosted_turn_before_the_next_model_call() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(budget_stop_hook_pauses_the_hosted_turn_inner());
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

async fn budget_stop_hook_pauses_the_hosted_turn_inner() {
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let model_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let model: Arc<dyn ChatModel<()>> = Arc::new(BudgetedToolModel(model_calls.clone()));
    let models = TurnModelSource::from_model(model)
        .build("claude-sonnet-4-6", 0.0, None, None)
        .expect("scripted turn models build");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut context = root_context("budget-stop", "/tmp/budget-stop", tx);
    let stop_reason = Arc::new(std::sync::Mutex::new(None));
    context.stop_hooks.push(Arc::new(RecordingBudgetStopHook {
        inner: BudgetStopHook::new(1.0),
        reason: Arc::clone(&stop_reason),
    }));

    let outcome = TurnSpec {
        run_context: context,
        turn_models: models,
        provider_id: "test".to_string(),
        model: "claude-sonnet-4-6",
        history: root_messages("budget-stop"),
        tool_sets: vec![Arc::new(vec![
            Box::new(LimitedTool(tool_calls.clone())) as Box<dyn Tool>
        ])],
        allowed: None,
        max_iterations: 3,
        context_window: None,
        run_queue: None,
        early_exit_tools: &[],
        pause_at_cap: false,
        max_output_tokens: None,
        context_mw: TurnContextMiddleware::default(),
        tool_policy: None,
        defer_turn_completed_to_caller: true,
        subagent_scope: None,
        deterministic_cacheable: false,
    }
    .run_hosted(hosted_base(), "main".to_string())
    .await
    .expect("budget pause returns the partial turn");

    assert_eq!(model_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(tool_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(outcome.tool_calls, 1);
    assert_eq!(
        stop_reason.lock().expect("stop reason lock").as_deref(),
        Some("turn cost $3.0000 reached cap $1.0000")
    );
}

#[test]
fn deadline_wind_down_pauses_the_hosted_turn_with_its_work_intact() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(deadline_wind_down_pauses_the_hosted_turn_inner());
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

/// A turn whose deadline is past its wind-down point (800s into a 900s
/// backstop) must not be dropped and must not keep calling the model: it
/// pauses after the tool round in flight and returns `Ok` with that round's
/// results. A blank terminal text with completed tool outcomes is exactly the
/// shape the session driver's grounded close turns into the user's answer
/// (`needs_final_close`), instead of the backstop discarding the turn.
async fn deadline_wind_down_pauses_the_hosted_turn_inner() {
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let model_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let model: Arc<dyn ChatModel<()>> = Arc::new(BudgetedToolModel(model_calls.clone()));
    let models = TurnModelSource::from_model(model)
        .build("claude-sonnet-4-6", 0.0, None, None)
        .expect("scripted turn models build");
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let mut context = root_context("deadline-wind-down", "/tmp/deadline-wind-down", tx);
    let now = std::time::Instant::now();
    context.turn_deadline = Some(tinyagents_harness::turn_deadline::TurnDeadline::new(
        now - std::time::Duration::from_secs(800),
        std::time::Duration::from_secs(900),
    ));

    let outcome = TurnSpec {
        run_context: context,
        turn_models: models,
        provider_id: "test".to_string(),
        model: "claude-sonnet-4-6",
        history: root_messages("deadline-wind-down"),
        tool_sets: vec![Arc::new(vec![
            Box::new(LimitedTool(tool_calls.clone())) as Box<dyn Tool>
        ])],
        allowed: None,
        max_iterations: 50,
        context_window: None,
        run_queue: None,
        early_exit_tools: &[],
        pause_at_cap: true,
        max_output_tokens: None,
        context_mw: TurnContextMiddleware::default(),
        tool_policy: None,
        defer_turn_completed_to_caller: true,
        subagent_scope: None,
        deterministic_cacheable: false,
    }
    .run_hosted(hosted_base(), "main".to_string())
    .await
    .expect("a wound-down turn returns its partial work, not an error");

    assert_eq!(
        model_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "no model call after the wind-down point"
    );
    assert_eq!(tool_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(outcome.tool_calls, 1);
    assert!(
        outcome.text.trim().is_empty(),
        "no final answer yet: the close writes it"
    );
    assert_eq!(
        outcome.tool_outcomes.len(),
        1,
        "the completed round is kept"
    );
    assert!(
        !outcome.hit_cap,
        "a deadline wind-down is not an iteration cap"
    );
}

#[test]
fn a_turn_before_its_wind_down_point_runs_to_its_final_answer() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(async {
                    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                    let model_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                    let model: Arc<dyn ChatModel<()>> =
                        Arc::new(BudgetedToolModel(model_calls.clone()));
                    let models = TurnModelSource::from_model(model)
                        .build("claude-sonnet-4-6", 0.0, None, None)
                        .expect("scripted turn models build");
                    let (tx, _rx) = tokio::sync::mpsc::channel(64);
                    let mut context = root_context("deadline-fresh", "/tmp/deadline-fresh", tx);
                    context.turn_deadline = Some(
                        tinyagents_harness::turn_deadline::TurnDeadline::starting_now(
                            std::time::Duration::from_secs(900),
                        ),
                    );
                    let outcome = TurnSpec {
                        run_context: context,
                        turn_models: models,
                        provider_id: "test".to_string(),
                        model: "claude-sonnet-4-6",
                        history: root_messages("deadline-fresh"),
                        tool_sets: vec![Arc::new(vec![
                            Box::new(LimitedTool(tool_calls.clone())) as Box<dyn Tool>
                        ])],
                        allowed: None,
                        max_iterations: 50,
                        context_window: None,
                        run_queue: None,
                        early_exit_tools: &[],
                        pause_at_cap: true,
                        max_output_tokens: None,
                        context_mw: TurnContextMiddleware::default(),
                        tool_policy: None,
                        defer_turn_completed_to_caller: true,
                        subagent_scope: None,
                        deterministic_cacheable: false,
                    }
                    .run_hosted(hosted_base(), "main".to_string())
                    .await
                    .expect("turn completes");
                    assert_eq!(model_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
                    assert_eq!(outcome.text, "done");
                });
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

#[derive(Clone, Copy)]
enum GoldenStop {
    Finishes,
    BudgetCap,
    DeadlineWindDown,
}

/// Run one scripted turn and render everything observable about how it
/// stopped as a stable JSON object.
async fn golden_turn(label: &str, stop: GoldenStop) -> serde_json::Value {
    let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let model_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let model: Arc<dyn ChatModel<()>> = Arc::new(BudgetedToolModel(model_calls.clone()));
    let models = TurnModelSource::from_model(model)
        .build("claude-sonnet-4-6", 0.0, None, None)
        .expect("scripted turn models build");
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let mut context = root_context(label, &format!("/tmp/{label}"), tx);
    let stop_reason = Arc::new(std::sync::Mutex::new(None));
    match stop {
        GoldenStop::Finishes => {}
        GoldenStop::BudgetCap => context.stop_hooks.push(Arc::new(RecordingBudgetStopHook {
            inner: BudgetStopHook::new(1.0),
            reason: Arc::clone(&stop_reason),
        })),
        GoldenStop::DeadlineWindDown => {
            context.turn_deadline = Some(tinyagents_harness::turn_deadline::TurnDeadline::new(
                std::time::Instant::now() - std::time::Duration::from_secs(800),
                std::time::Duration::from_secs(900),
            ));
        }
    }
    let sidecar = context.session_sidecar.clone();
    let outcome = TurnSpec {
        run_context: context,
        turn_models: models,
        provider_id: "test".to_string(),
        model: "claude-sonnet-4-6",
        history: root_messages(label),
        tool_sets: vec![Arc::new(vec![
            Box::new(LimitedTool(tool_calls.clone())) as Box<dyn Tool>
        ])],
        allowed: None,
        max_iterations: 50,
        context_window: None,
        run_queue: None,
        early_exit_tools: &[],
        pause_at_cap: matches!(stop, GoldenStop::Finishes | GoldenStop::DeadlineWindDown),
        max_output_tokens: None,
        context_mw: TurnContextMiddleware::default(),
        tool_policy: None,
        defer_turn_completed_to_caller: true,
        subagent_scope: None,
        deterministic_cacheable: false,
    }
    .run_hosted(hosted_base(), "main".to_string())
    .await
    .expect("scripted turn returns its outcome");
    let wind_down = sidecar
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .wind_down;
    serde_json::json!({
        "scenario": label,
        "model_calls": outcome.model_calls,
        "tool_calls": outcome.tool_calls,
        "text": outcome.text,
        "hit_cap": outcome.hit_cap,
        "tool_outcomes": outcome.tool_outcomes.len(),
        "stop_reason": stop_reason.lock().expect("stop reason lock").clone(),
        "wind_down_recorded": wind_down,
        "history": outcome
            .history
            .iter()
            .map(|m| format!("{m:?}"))
            .collect::<Vec<_>>(),
    })
}

/// Pins the observable stop behaviour of a scripted turn over the host's
/// driver path: final answer, budget-cap pause (with its stop reason) and
/// deadline wind-down. Regenerate with `UPDATE_TURN_STOP_GOLDEN=1` only when a
/// behaviour change is intended.
#[test]
fn turn_stop_transcripts_match_golden() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(async {
                    let out = vec![
                        golden_turn("golden-final", GoldenStop::Finishes).await,
                        golden_turn("golden-budget", GoldenStop::BudgetCap).await,
                        golden_turn("golden-wind-down", GoldenStop::DeadlineWindDown).await,
                    ];
                    let actual = serde_json::to_string_pretty(&serde_json::Value::Array(out))
                        .unwrap()
                        + "\n";
                    let path = concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/src/agent/tinyagents/fixtures/turn_stop_golden.json"
                    );
                    if std::env::var_os("UPDATE_TURN_STOP_GOLDEN").is_some() {
                        std::fs::write(path, &actual).unwrap();
                        return;
                    }
                    assert_eq!(
                        actual,
                        include_str!("fixtures/turn_stop_golden.json"),
                        "turn stop transcript changed"
                    );
                });
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}
