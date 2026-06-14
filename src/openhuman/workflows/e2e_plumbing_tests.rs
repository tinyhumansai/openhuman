//! End-to-end "plumbing" tests for the unified workflow primitive.
//!
//! These confirm the wiring the skills→workflows unification introduced,
//! using a seeded workspace + a **mock LLM** (no network):
//!
//!   A. create → registry round-trip — the combo create form's two halves
//!      (`when_to_use` trigger from the old agent-workflow + declared
//!      `[[inputs]]` from the skill form) both persist and are read back by
//!      the registry the orchestrator discovers through.
//!   B. mock-LLM orchestrator turn — a scripted model drives the real agent
//!      loop to call `list_workflows` (sees the seeded workflow) and
//!      `run_workflow` (unknown id → clean tool error), proving both tools
//!      are wired into the orchestrator's tool surface and execute.
//!   C. `await_run_outcome` — the runtime polls a run's log to its terminal
//!      footer (the `run_workflow` await), and auto-detaches (returns `None`)
//!      when the run outlives the wait budget.
//!
//! NOTE on scope: the autonomous run `run_workflow` spawns builds its provider
//! from config (`create_chat_provider`), which has no test-injection seam, so
//! these tests deliberately do not drive an inner run to DONE via a mock LLM —
//! that generic autonomous-run path is covered by the subagent_runner suite.
//! Here we confirm everything up to and around that boundary.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::openhuman::agent::harness::run_tool_call_loop;
use crate::openhuman::agent::tools::RunWorkflowTool;
use crate::openhuman::config::{Config, MultimodalConfig, MultimodalFileConfig};
use crate::openhuman::inference::provider::traits::{ChatMessage, ProviderCapabilities};
use crate::openhuman::inference::provider::{ChatRequest, ChatResponse, Provider, ToolCall};
use crate::openhuman::skill_runtime::await_run_outcome;
use crate::openhuman::tools::policy::DefaultToolPolicy;
use crate::openhuman::tools::traits::Tool;
use crate::openhuman::workflows::ops_create::{
    create_workflow_inner, CreateWorkflowParams, WorkflowCreateInputDef,
};
use crate::openhuman::workflows::ops_types::WorkflowScope;
use crate::openhuman::workflows::registry::get_workflow;
use crate::openhuman::workflows::run_log;

// ── Mock LLM ─────────────────────────────────────────────────────────────
// Minimal scripted provider: pops queued ChatResponses in order. Mirrors the
// one in `agent/harness/tool_loop_tests.rs` (kept local so this file is
// self-contained).
struct ScriptedProvider {
    responses: Mutex<Vec<anyhow::Result<ChatResponse>>>,
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("fallback".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.responses.lock().remove(0)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            ..ProviderCapabilities::default()
        }
    }
}

fn tool_call(id: &str, name: &str, args: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args.to_string(),
            extra_content: None,
        }],
        usage: None,
        reasoning_content: None,
    }
}

fn final_text(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }
}

/// Seed a trusted project-scope workflow directly on disk (the discovery tools
/// scan `<ws>/.openhuman/skills` for project workflows when the trust marker is
/// present — hermetic, independent of the real home dir).
fn seed_project_workflow(ws: &std::path::Path, slug: &str, description: &str) {
    std::fs::create_dir_all(ws.join(".openhuman")).unwrap();
    std::fs::write(ws.join(".openhuman").join("trust"), "").unwrap();
    let dir = ws.join(".openhuman").join("skills").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {slug}\ndescription: {description}\n---\n\n{description}\n"),
    )
    .unwrap();
}

// ── A. create → registry round-trip (the combo persists + is RUNNABLE) ───

// Regression guard for the create→run unification: a workflow authored via the
// create path (`create_workflow_inner` → `.openhuman/skills`) must be found by the
// RUN path's `get_workflow` (→ `load_workflows`, now reading the same roots as
// `discover_workflows`), with its `when_to_use` trigger + declared inputs intact.
// Before the loader unification this failed ("unknown workflow") because the
// run path scanned only `<ws>/skills`. Hermetic (project scope + temp dir).
#[test]
fn create_then_registry_roundtrip_preserves_when_to_use_and_inputs() {
    let ws = tempfile::tempdir().unwrap();
    // Project scope keeps it hermetic (writes under the workspace, not $HOME).
    std::fs::create_dir_all(ws.path().join(".openhuman")).unwrap();
    std::fs::write(ws.path().join(".openhuman").join("trust"), "").unwrap();

    let params = CreateWorkflowParams {
        name: "Triage Inbox".to_string(),
        description: "Summarise and label the inbox.".to_string(),
        when_to_use: Some("when the user asks to triage email".to_string()),
        scope: WorkflowScope::Project,
        inputs: vec![WorkflowCreateInputDef {
            name: "label".to_string(),
            description: Some("Gmail label to apply".to_string()),
            required: false,
            type_: None,
        }],
        ..Default::default()
    };
    let created =
        create_workflow_inner(None, ws.path(), params).expect("create workflow should succeed");

    // The registry (what the orchestrator's discovery reads) must surface BOTH
    // halves of the unified form.
    let def = get_workflow(ws.path(), &created.name).expect("created workflow is discoverable");
    assert_eq!(
        def.definition.when_to_use, "when the user asks to triage email",
        "the workflow's trigger (when_to_use) must round-trip, distinct from the description"
    );
    assert_eq!(def.inputs.len(), 1, "the declared input must round-trip");
    assert_eq!(def.inputs[0].name, "label");
}

// ── B. mock-LLM orchestrator drives the workflow tools ───────────────────

#[tokio::test]
async fn mock_llm_orchestrator_lists_and_runs_workflows_through_the_loop() {
    let ws = tempfile::tempdir().unwrap();
    seed_project_workflow(ws.path(), "triage-inbox", "Summarise the inbox.");

    let mut config = Config::default();
    config.workspace_dir = ws.path().to_path_buf();
    let config = Arc::new(config);

    // The two tools the orchestrator now carries for workflows.
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(crate::openhuman::workflows::tools::WorkflowListTool::new(
            config.clone(),
        )),
        Box::new(RunWorkflowTool::new()),
    ];

    // Scripted: discover → attempt to run an unknown workflow → wrap up.
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![
            Ok(tool_call("c1", "list_workflows", serde_json::json!({}))),
            Ok(tool_call(
                "c2",
                "run_workflow",
                // wait_seconds: 0 = fire-and-forget; unknown id fails preflight
                // synchronously (deterministic regardless of the global workspace).
                serde_json::json!({ "workflow_id": "ghost-workflow", "wait_seconds": 0 }),
            )),
            Ok(final_text("done")),
        ]),
    };

    let mut history = vec![ChatMessage::user("Triage my inbox using a workflow.")];
    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "test-provider",
        "model",
        0.0,
        true,
        "channel",
        &MultimodalConfig::default(),
        &MultimodalFileConfig::default(),
        5,
        None,
        None,
        &[],
        None,
        None,
        &DefaultToolPolicy,
    )
    .await
    .expect("tool loop should run to completion");

    assert_eq!(result, "done");

    let tool_msgs: String = history
        .iter()
        .filter(|m| m.role == "tool")
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n");

    // list_workflows executed against the seeded workspace and surfaced it.
    assert!(
        tool_msgs.contains("triage-inbox"),
        "list_workflows must return the seeded workflow; got:\n{tool_msgs}"
    );
    // run_workflow is wired into the orchestrator loop and its error path
    // (unknown workflow) flows back as a tool result the model can read.
    assert!(
        tool_msgs.contains("ghost-workflow"),
        "run_workflow's unknown-workflow error must reach the loop; got:\n{tool_msgs}"
    );
}

// ── C. await_run_outcome: terminal poll + auto-detach ────────────────────

#[tokio::test]
async fn await_run_outcome_returns_terminal_footer_then_auto_detaches() {
    let ws = tempfile::tempdir().unwrap();

    // A finished run: header + DONE footer → await returns the outcome.
    let done = run_log::run_log_path(ws.path(), "triage-inbox", "run-done-1234");
    std::fs::create_dir_all(done.parent().unwrap()).unwrap();
    run_log::write_header(
        &done,
        "triage-inbox",
        "run-done-1234",
        &serde_json::json!({}),
        "task",
    )
    .await
    .unwrap();
    run_log::write_footer(&done, "DONE", 1234, "inbox triaged: 12 archived")
        .await
        .unwrap();
    let outcome = await_run_outcome(&done, Duration::from_secs(2))
        .await
        .expect("terminal footer must be returned");
    assert_eq!(outcome.status, "DONE");
    assert!(outcome.output.contains("inbox triaged"));

    // A still-running run (header, no footer): await up to a short budget then
    // auto-detach with None (what run_workflow turns into a run_id handle).
    let running = run_log::run_log_path(ws.path(), "triage-inbox", "run-live-5678");
    run_log::write_header(
        &running,
        "triage-inbox",
        "run-live-5678",
        &serde_json::json!({}),
        "task",
    )
    .await
    .unwrap();
    let detached = await_run_outcome(&running, Duration::from_millis(300)).await;
    assert!(
        detached.is_none(),
        "a run with no terminal footer must auto-detach (None) past the wait budget"
    );
}
