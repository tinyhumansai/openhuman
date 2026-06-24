//! Engine unit tests (#3375 PR2).
//!
//! These exercise the phase scheduler ([`super::drive_phases`]) directly with a
//! mock `Provider` so child agents resolve deterministically and never touch the
//! network. The full [`super::start_workflow_run`] entry point (which builds a
//! real `Agent` from config) is covered by the JSON-RPC e2e test over the live
//! core stack with the mock backend.
//!
//! Test strategy: install a mock [`ParentExecutionContext`] via
//! [`with_parent_context`] (mirroring `agent_orchestration::ops_tests`), seed a
//! `Running` workflow-run row in a tempdir-backed ledger, then drive a custom
//! definition and assert on the persisted `phase_states` + run status.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

use super::*;
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::openhuman::config::{AgentConfig, Config};
use crate::openhuman::context::prompt::ToolCallFormat;
use crate::openhuman::inference::provider::traits::ProviderCapabilities;
use crate::openhuman::inference::provider::{ChatRequest, ChatResponse, Provider};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use crate::openhuman::session_db::run_ledger::{
    get_workflow_run, upsert_workflow_run, WorkflowRunUpsert,
};
use crate::openhuman::tools::{Tool, ToolSpec};

use super::super::types::{WorkflowDefinition, WorkflowPhase, WorkflowSafetyTier};

// ── Mocks (mirrors agent_orchestration::ops_tests) ──────────────────────────

#[derive(Default)]
struct NoopMemory;

#[async_trait]
impl Memory for NoopMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }
    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }
    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }
    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }
    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }
    async fn health_check(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "noop"
    }
}

fn text_response(text: impl Into<String>) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: Vec::new(),
        usage: None,
        reasoning_content: None,
    }
}

/// Mock provider that records peak concurrency and answers each child with a
/// short deterministic completion. Sleeps briefly so overlapping spawns are
/// observable for the concurrency-cap assertions.
#[derive(Clone, Default)]
struct PeakProvider {
    calls: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
    fail_on: Arc<Mutex<Option<String>>>,
}

impl PeakProvider {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }
    fn fail_when_prompt_contains(&self, needle: &str) {
        *self.fail_on.lock() = Some(needle.to_string());
    }
    fn record_peak(&self, current: usize) {
        let mut observed = self.max_active.load(Ordering::SeqCst);
        while current > observed {
            match self.max_active.compare_exchange(
                observed,
                current,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(next) => observed = next,
            }
        }
    }
}

#[async_trait]
impl Provider for PeakProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("ok".to_string())
    }
    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let current = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.record_peak(current);
        sleep(Duration::from_millis(40)).await;
        let flattened = request
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.prompts.lock().push(flattened.clone());
        self.active.fetch_sub(1, Ordering::SeqCst);

        if let Some(needle) = self.fail_on.lock().as_ref() {
            if flattened.contains(needle.as_str()) {
                return Err(anyhow::anyhow!("mock provider forced failure"));
            }
        }
        Ok(text_response("PHASE_OUTPUT_OK"))
    }
}

fn mock_parent(provider: Arc<dyn Provider>) -> ParentExecutionContext {
    ParentExecutionContext {
        agent_definition_id: "workflow_engine".to_string(),
        allowed_subagent_ids: HashSet::new(),
        provider,
        all_tools: Arc::new(Vec::<Box<dyn Tool>>::new()),
        all_tool_specs: Arc::new(Vec::<ToolSpec>::new()),
        visible_tool_names: std::collections::HashSet::new(),
        model_name: "test-model".to_string(),
        temperature: 0.2,
        workspace_dir: std::env::temp_dir(),
        memory: Arc::new(NoopMemory),
        agent_config: AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "workflow-engine-test".to_string(),
        channel: "test".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "0_workflow_engine".to_string(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

fn test_config() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, config)
}

/// Seed a `Running` workflow run row for `definition`, returning its id.
fn seed_run(config: &Config, definition: &WorkflowDefinition, input: Value) -> String {
    let id = format!("wfrun-test-{}", uuid::Uuid::new_v4());
    upsert_workflow_run(
        config,
        WorkflowRunUpsert {
            id: id.clone(),
            definition_id: definition.id.clone(),
            parent_thread_id: None,
            input,
            phase_states: init_phase_states(definition),
            child_run_ids: Vec::new(),
            status: WorkflowRunStatus::Running,
            summary: None,
            started_at: None,
            completed_at: None,
        },
    )
    .expect("seed run");
    id
}

/// A linear A→B→C definition using `code_executor` (a builtin leaf agent), with
/// `concurrency` workers in phase B.
fn linear_def(concurrency: u32, max_children: u32, parallel_in_b: usize) -> WorkflowDefinition {
    WorkflowDefinition {
        id: "test_linear".to_string(),
        name: "test linear".to_string(),
        description: "A then parallel B then C".to_string(),
        phases: vec![
            WorkflowPhase {
                name: "a".to_string(),
                description: "phase a".to_string(),
                agent_ids: vec!["code_executor".to_string()],
                depends_on: vec![],
            },
            WorkflowPhase {
                name: "b".to_string(),
                description: "phase b PARALLEL".to_string(),
                agent_ids: vec!["code_executor".to_string(); parallel_in_b],
                depends_on: vec!["a".to_string()],
            },
            WorkflowPhase {
                name: "c".to_string(),
                description: "phase c synthesize".to_string(),
                agent_ids: vec!["code_executor".to_string()],
                depends_on: vec!["b".to_string()],
            },
        ],
        default_concurrency: concurrency,
        max_children,
        safety_tier: WorkflowSafetyTier::ReadOnly,
    }
}

fn status_of(config: &Config, id: &str) -> WorkflowRunStatus {
    get_workflow_run(config, id).unwrap().unwrap().status
}

fn phase_states(config: &Config, id: &str) -> Value {
    get_workflow_run(config, id).unwrap().unwrap().phase_states
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn unit_phases_execute_in_dependency_order() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    let provider = PeakProvider::default();
    let def = linear_def(2, 8, 2);
    let id = seed_run(
        &config,
        &def,
        json!({ "question": "what is X?", "modelOverride": "test-model" }),
    );
    let cancel = Arc::new(AtomicBool::new(false));

    with_parent_context(mock_parent(Arc::new(provider.clone())), async {
        drive_phases(&config, &id, &def, &cancel).await
    })
    .await
    .expect("drive_phases ok");

    assert_eq!(status_of(&config, &id), WorkflowRunStatus::Completed);
    let states = phase_states(&config, &id);
    for phase in ["a", "b", "c"] {
        assert_eq!(
            states
                .get(phase)
                .and_then(|p| p.get("status"))
                .and_then(Value::as_str),
            Some("completed"),
            "phase {phase} should be completed: {states}"
        );
    }
    // 1 (a) + 2 (b) + 1 (c) = 4 children executed.
    assert_eq!(provider.calls(), 4, "all four children should run");

    // Summary comes from the last phase's output (no phase literally named
    // 'synthesize', so the fallback picks phase c).
    let summary = get_workflow_run(&config, &id).unwrap().unwrap().summary;
    assert!(
        summary
            .as_deref()
            .unwrap_or_default()
            .contains("PHASE_OUTPUT_OK"),
        "summary should carry final phase output: {summary:?}"
    );
}

#[tokio::test]
async fn unit_concurrency_cap_is_respected() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    let provider = PeakProvider::default();
    // 4 parallel workers in phase b, but concurrency capped at 2.
    let def = linear_def(2, 16, 4);
    let id = seed_run(
        &config,
        &def,
        json!({ "question": "q", "modelOverride": "test-model" }),
    );
    let cancel = Arc::new(AtomicBool::new(false));

    with_parent_context(mock_parent(Arc::new(provider.clone())), async {
        drive_phases(&config, &id, &def, &cancel).await
    })
    .await
    .expect("drive_phases ok");

    assert_eq!(status_of(&config, &id), WorkflowRunStatus::Completed);
    assert!(
        provider.max_active() <= 2,
        "concurrency cap of 2 exceeded: peak={}",
        provider.max_active()
    );
}

#[tokio::test]
async fn unit_max_children_hard_cap_fails_run() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    let provider = PeakProvider::default();
    // a(1) + b(4) = 5 needed, but max_children = 3 → run fails in phase b.
    let def = linear_def(2, 3, 4);
    let id = seed_run(
        &config,
        &def,
        json!({ "question": "q", "modelOverride": "test-model" }),
    );
    let cancel = Arc::new(AtomicBool::new(false));

    with_parent_context(mock_parent(Arc::new(provider.clone())), async {
        drive_phases(&config, &id, &def, &cancel).await
    })
    .await
    .expect("drive_phases returns Ok with terminal Failed state");

    assert_eq!(status_of(&config, &id), WorkflowRunStatus::Failed);
    let summary = get_workflow_run(&config, &id).unwrap().unwrap().summary;
    assert!(
        summary
            .as_deref()
            .unwrap_or_default()
            .contains("max_children"),
        "failure reason should mention max_children: {summary:?}"
    );
    // Phase a completed before the cap bit; partial state preserved.
    let states = phase_states(&config, &id);
    assert_eq!(
        states
            .get("a")
            .and_then(|p| p.get("status"))
            .and_then(Value::as_str),
        Some("completed")
    );
}

#[tokio::test]
async fn unit_failed_child_marks_run_failed_with_partial_state() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    let provider = PeakProvider::default();
    // Force the phase-b child to fail (its prompt mentions "PARALLEL").
    provider.fail_when_prompt_contains("phase b PARALLEL");
    let def = linear_def(2, 8, 1);
    let id = seed_run(
        &config,
        &def,
        json!({ "question": "q", "modelOverride": "test-model" }),
    );
    let cancel = Arc::new(AtomicBool::new(false));

    with_parent_context(mock_parent(Arc::new(provider.clone())), async {
        drive_phases(&config, &id, &def, &cancel).await
    })
    .await
    .expect("drive_phases returns Ok with terminal Failed state");

    assert_eq!(status_of(&config, &id), WorkflowRunStatus::Failed);
    let states = phase_states(&config, &id);
    assert_eq!(
        states
            .get("a")
            .and_then(|p| p.get("status"))
            .and_then(Value::as_str),
        Some("completed"),
        "phase a should have completed before b failed"
    );
    assert_eq!(
        states
            .get("b")
            .and_then(|p| p.get("status"))
            .and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        states
            .get("c")
            .and_then(|p| p.get("status"))
            .and_then(Value::as_str),
        Some("pending"),
        "phase c never ran"
    );
}

#[tokio::test]
async fn unit_stop_mid_run_marks_interrupted() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    let provider = PeakProvider::default();
    let def = linear_def(2, 8, 1);
    let id = seed_run(
        &config,
        &def,
        json!({ "question": "q", "modelOverride": "test-model" }),
    );
    // Pre-flip the cancel flag so the loop interrupts before the first phase.
    let cancel = Arc::new(AtomicBool::new(true));

    with_parent_context(mock_parent(Arc::new(provider.clone())), async {
        drive_phases(&config, &id, &def, &cancel).await
    })
    .await
    .expect("drive_phases ok");

    assert_eq!(status_of(&config, &id), WorkflowRunStatus::Interrupted);
    // No children ran — cancellation was checked before the first phase.
    assert_eq!(provider.calls(), 0);
}

#[tokio::test]
async fn unit_resume_skips_completed_phases() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    let provider = PeakProvider::default();
    let def = linear_def(2, 8, 1);
    let id = seed_run(
        &config,
        &def,
        json!({ "question": "q", "modelOverride": "test-model" }),
    );

    // Mark phase 'a' already completed (simulating a prior partial run).
    let run = get_workflow_run(&config, &id).unwrap().unwrap();
    let mut states = run.phase_states.clone();
    states["a"]["status"] = json!("completed");
    states["a"]["outputs"] =
        json!([{ "orchestrationId": "x", "agentId": "code_executor", "output": "A_DONE" }]);
    upsert_workflow_run(
        &config,
        WorkflowRunUpsert {
            id: id.clone(),
            definition_id: run.definition_id.clone(),
            parent_thread_id: None,
            input: run.input.clone(),
            phase_states: states,
            child_run_ids: Vec::new(),
            status: WorkflowRunStatus::Running,
            summary: None,
            started_at: Some(run.started_at),
            completed_at: None,
        },
    )
    .unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    with_parent_context(mock_parent(Arc::new(provider.clone())), async {
        drive_phases(&config, &id, &def, &cancel).await
    })
    .await
    .expect("drive_phases ok");

    assert_eq!(status_of(&config, &id), WorkflowRunStatus::Completed);
    // Only phases b and c run on resume (a was already completed): 2 calls.
    assert_eq!(
        provider.calls(),
        2,
        "resume must skip the completed phase a"
    );
}
