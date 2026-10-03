//! Hosted-turn integration coverage for the per-turn budget stop hook.

use super::*;
use crate::agent::stop_hooks::{BudgetStopHook, StopDecision, StopHook, TurnState};
use crate::agent::tinyagents::TurnModelSource;
use async_trait::async_trait;
use std::sync::Arc;
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
impl StopHook for RecordingBudgetStopHook {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn check(&self, state: &TurnState<'_>) -> StopDecision {
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
            crate::agent::harness::definition::AgentDefinitionRegistry::builtins_only(),
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
        .build("root-test-model", 0.0, None, None)
        .expect("scripted turn models build");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut context = root_context("budget-stop", "/tmp/budget-stop", tx);
    let stop_reason = Arc::new(std::sync::Mutex::new(None));
    context.stop_hooks.push(Arc::new(RecordingBudgetStopHook {
        inner: BudgetStopHook::new(1.0),
        reason: Arc::clone(&stop_reason),
    }));

    let outcome = run_root_turn_via_hosted_agent(
        context,
        hosted_base(),
        "main".to_string(),
        models,
        "test".to_string(),
        "root-test-model",
        root_messages("budget-stop"),
        vec![Arc::new(vec![
            Box::new(LimitedTool(tool_calls.clone())) as Box<dyn Tool>
        ])],
        None,
        3,
        None,
        None,
        &[],
        false,
        None,
        TurnContextMiddleware::default(),
        None,
        true,
    )
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
