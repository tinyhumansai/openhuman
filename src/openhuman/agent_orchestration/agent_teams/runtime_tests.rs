//! Live-runtime unit tests (#3374 PR4).
//!
//! The worker-driving core ([`super::drive_member`]) is exercised directly with
//! a mock `Provider` installed via [`with_parent_context`] (mirroring
//! `workflow_runs::engine_tests`), so a teammate runs deterministically without
//! touching the network. [`super::start_member_run`]'s pre-spawn outcome routing
//! (blocked / already-claimed / no-claimable / unknown) is tested through the
//! real entry point; its `Started` path (which spawns a loop building a real
//! `Agent` from config) is covered by the JSON-RPC e2e over the live core stack.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::*;
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::openhuman::config::{AgentConfig, Config};
use crate::openhuman::context::prompt::ToolCallFormat;
use crate::openhuman::inference::provider::traits::ProviderCapabilities;
use crate::openhuman::inference::provider::{ChatRequest, ChatResponse, Provider};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use crate::openhuman::session_db::run_ledger::{
    self, AgentTeamMemberStatus, AgentTeamMemberUpsert, AgentTeamStatus, AgentTeamTaskStatus,
    AgentTeamTaskUpsert, AgentTeamUpsert,
};
use crate::openhuman::tools::{Tool, ToolSpec};

// ── Mocks (mirror workflow_runs::engine_tests) ──────────────────────────────

#[derive(Default)]
struct NoopMemory;

#[async_trait]
impl Memory for NoopMemory {
    async fn store(
        &self,
        _ns: &str,
        _key: &str,
        _content: &str,
        _cat: MemoryCategory,
        _sid: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recall(
        &self,
        _q: &str,
        _l: usize,
        _o: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }
    async fn get(&self, _ns: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }
    async fn list(
        &self,
        _ns: Option<&str>,
        _cat: Option<&MemoryCategory>,
        _sid: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }
    async fn forget(&self, _ns: &str, _key: &str) -> anyhow::Result<bool> {
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

/// Mock provider that answers every child with a fixed completion, or fails.
#[derive(Clone)]
struct CannedProvider {
    output: String,
    fail: bool,
}

#[async_trait]
impl Provider for CannedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok("ok".to_string())
    }
    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        if self.fail {
            return Err(anyhow::anyhow!("mock provider forced failure"));
        }
        Ok(text_response(self.output.clone()))
    }
}

fn mock_parent(provider: Arc<dyn Provider>) -> ParentExecutionContext {
    ParentExecutionContext {
        agent_definition_id: "agent_team_runtime".to_string(),
        allowed_subagent_ids: HashSet::new(),
        provider,
        all_tools: Arc::new(Vec::<Box<dyn Tool>>::new()),
        all_tool_specs: Arc::new(Vec::<ToolSpec>::new()),
        visible_tool_names: std::collections::HashSet::new(),
        model_name: "test-model".to_string(),
        temperature: 0.0,
        workspace_dir: std::env::temp_dir(),
        memory: Arc::new(NoopMemory),
        agent_config: AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "team-runtime-test".to_string(),
        channel: "test".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "0_agent_team_runtime".to_string(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

fn test_config() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        action_dir: dir.path().join("actions"),
        ..Config::default()
    };
    (dir, config)
}

fn seed_team(config: &Config, team_id: &str) {
    run_ledger::upsert_agent_team(
        config,
        AgentTeamUpsert {
            id: team_id.into(),
            parent_thread_id: None,
            lead_agent_id: "lead".into(),
            status: AgentTeamStatus::Active,
            summary: None,
            created_at: None,
            closed_at: None,
        },
    )
    .unwrap();
}

fn seed_member(config: &Config, team_id: &str, member_id: &str, agent_id: Option<&str>) {
    run_ledger::upsert_agent_team_member(
        config,
        AgentTeamMemberUpsert {
            id: member_id.into(),
            team_id: team_id.into(),
            name: member_id.into(),
            agent_id: agent_id.map(str::to_string),
            member_status: AgentTeamMemberStatus::Idle,
            current_task_id: None,
            worker_thread_id: None,
            run_id: None,
            created_at: None,
        },
    )
    .unwrap();
}

fn seed_task(
    config: &Config,
    team_id: &str,
    task_id: &str,
    status: AgentTeamTaskStatus,
    owner: Option<&str>,
    depends_on: Vec<String>,
) {
    run_ledger::upsert_agent_team_task(
        config,
        AgentTeamTaskUpsert {
            id: task_id.into(),
            team_id: team_id.into(),
            title: format!("task {task_id}"),
            objective: Some(format!("do {task_id}")),
            status,
            owner_member_id: owner.map(str::to_string),
            depends_on,
            gate_status: None,
            gate_reason: None,
            evidence: vec![],
            source_run_id: None,
            order_index: 0,
            created_at: None,
        },
    )
    .unwrap();
}

// ── drive_member (the live-exec core) ───────────────────────────────────────

#[tokio::test]
async fn drive_member_completes_task_with_worker_output_as_evidence() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("code_executor"));
    seed_task(
        &config,
        "team-1",
        "task-a",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    // Claim + mark running, mirroring what start_member_run does pre-spawn.
    run_ledger::claim_agent_team_task(&config, "team-1", "task-a", "m1", "teamrun-x").unwrap();
    run_ledger::mark_agent_team_member_running(
        &config,
        "team-1",
        "m1",
        "task-a",
        "teamrun-x",
        "teamrun-x",
    )
    .unwrap();
    let task = run_ledger::get_agent_team_task(&config, "task-a")
        .unwrap()
        .unwrap();

    let provider = Arc::new(CannedProvider {
        output: "did the thing".into(),
        fail: false,
    });
    with_parent_context(mock_parent(provider), async {
        drive_member(
            &config,
            "team-1",
            "m1",
            "code_executor",
            &task,
            "teamrun-x",
            Some("test-model".into()),
        )
        .await
    })
    .await
    .expect("drive_member ok");

    let done = run_ledger::get_agent_team_task(&config, "task-a")
        .unwrap()
        .unwrap();
    assert_eq!(done.status, AgentTeamTaskStatus::Done);
    assert_eq!(done.gate_status, "passed");
    assert_eq!(done.evidence.len(), 1, "worker output captured as evidence");
    assert!(done.evidence[0].contains("teamrun-x"));

    let member = run_ledger::get_agent_team_member(&config, "m1")
        .unwrap()
        .unwrap();
    assert_eq!(member.member_status, AgentTeamMemberStatus::Idle);
    assert_eq!(member.current_task_id, None);
}

#[tokio::test]
async fn drive_member_releases_task_when_worker_fails() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("code_executor"));
    seed_task(
        &config,
        "team-1",
        "task-a",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    run_ledger::claim_agent_team_task(&config, "team-1", "task-a", "m1", "teamrun-x").unwrap();
    run_ledger::mark_agent_team_member_running(
        &config,
        "team-1",
        "m1",
        "task-a",
        "teamrun-x",
        "teamrun-x",
    )
    .unwrap();
    let task = run_ledger::get_agent_team_task(&config, "task-a")
        .unwrap()
        .unwrap();

    let provider = Arc::new(CannedProvider {
        output: String::new(),
        fail: true,
    });
    with_parent_context(mock_parent(provider), async {
        drive_member(
            &config,
            "team-1",
            "m1",
            "code_executor",
            &task,
            "teamrun-x",
            Some("test-model".into()),
        )
        .await
    })
    .await
    .expect("drive_member handles worker failure without erroring");

    // Task released back to todo, claim cleared → reclaimable.
    let released = run_ledger::get_agent_team_task(&config, "task-a")
        .unwrap()
        .unwrap();
    assert_eq!(released.status, AgentTeamTaskStatus::Todo);
    assert_eq!(released.claimed_by_member_id, None);

    let member = run_ledger::get_agent_team_member(&config, "m1")
        .unwrap()
        .unwrap();
    assert_eq!(member.member_status, AgentTeamMemberStatus::Idle);
}

// ── start_member_run pre-spawn outcome routing ──────────────────────────────

#[tokio::test]
async fn start_member_run_blocks_on_unmet_dependency() {
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("code_executor"));
    seed_task(
        &config,
        "team-1",
        "task-a",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    seed_task(
        &config,
        "team-1",
        "task-b",
        AgentTeamTaskStatus::Todo,
        None,
        vec!["task-a".into()],
    );

    // Explicit task-b: its dep task-a is not done → Blocked, no spawn.
    let outcome = start_member_run(&config, "team-1", "m1", Some("task-b"), None)
        .await
        .unwrap();
    match outcome {
        StartMemberOutcome::Blocked { unmet } => assert_eq!(unmet, vec!["task-a".to_string()]),
        other => panic!("expected Blocked, got {other:?}"),
    }
}

#[tokio::test]
async fn start_member_run_reports_already_claimed() {
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("code_executor"));
    seed_member(&config, "team-1", "m2", Some("code_executor"));
    seed_task(
        &config,
        "team-1",
        "task-a",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    // m2 already holds task-a.
    run_ledger::claim_agent_team_task(&config, "team-1", "task-a", "m2", "tok").unwrap();

    let outcome = start_member_run(&config, "team-1", "m1", Some("task-a"), None)
        .await
        .unwrap();
    assert_eq!(outcome, StartMemberOutcome::AlreadyClaimed);
}

#[tokio::test]
async fn start_member_run_no_claimable_and_unknown_task() {
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("code_executor"));
    // No tasks at all → nothing claimable.
    let none = start_member_run(&config, "team-1", "m1", None, None)
        .await
        .unwrap();
    assert_eq!(none, StartMemberOutcome::NoClaimableTask);

    // Explicit unknown task id → UnknownTask.
    let unknown = start_member_run(&config, "team-1", "m1", Some("ghost"), None)
        .await
        .unwrap();
    assert_eq!(unknown, StartMemberOutcome::UnknownTask);

    // Unknown member → typed error.
    let err = start_member_run(&config, "team-1", "ghost", None, None)
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast::<TeamError>().unwrap(),
        TeamError::UnknownMember {
            member_id: "ghost".into()
        }
    );
}

#[tokio::test]
async fn start_member_run_rejects_already_active_member_without_side_effects() {
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    // A member already mid-run (active), plus a fresh claimable task.
    run_ledger::upsert_agent_team_member(
        &config,
        AgentTeamMemberUpsert {
            id: "m1".into(),
            team_id: "team-1".into(),
            name: "m1".into(),
            agent_id: Some("researcher".into()),
            member_status: AgentTeamMemberStatus::Active,
            current_task_id: Some("t-running".into()),
            worker_thread_id: Some("run-running".into()),
            run_id: Some("run-running".into()),
            created_at: None,
        },
    )
    .unwrap();
    seed_task(
        &config,
        "team-1",
        "t-free",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );

    let outcome = start_member_run(&config, "team-1", "m1", None, None)
        .await
        .unwrap();
    assert_eq!(outcome, StartMemberOutcome::AlreadyActive);

    // No claim happened — the free task is untouched and the member still points
    // at its original run (no clobbered pointer).
    let task = run_ledger::get_agent_team_task(&config, "t-free")
        .unwrap()
        .expect("task exists");
    assert_eq!(task.status, AgentTeamTaskStatus::Todo);
    assert!(task.claimed_by_member_id.is_none());
    let member = run_ledger::get_agent_team_member(&config, "m1")
        .unwrap()
        .expect("member exists");
    assert_eq!(member.current_task_id.as_deref(), Some("t-running"));
    assert_eq!(member.run_id.as_deref(), Some("run-running"));
}

// ── helpers ─────────────────────────────────────────────────────────────────

#[test]
fn pick_claimable_respects_deps_ownership_and_claim() {
    let (_dir, config) = test_config();
    let _ = &config;
    let tasks = vec![
        // done dep
        AgentTeamTask {
            id: "a".into(),
            team_id: "t".into(),
            title: "a".into(),
            objective: None,
            status: AgentTeamTaskStatus::Done,
            owner_member_id: None,
            claimed_by_member_id: Some("m1".into()),
            claim_token: Some("x".into()),
            depends_on: vec![],
            gate_status: "passed".into(),
            gate_reason: None,
            evidence: vec![],
            source_run_id: None,
            order_index: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        // owned by m2 → not claimable by m1
        AgentTeamTask {
            id: "b".into(),
            team_id: "t".into(),
            title: "b".into(),
            objective: None,
            status: AgentTeamTaskStatus::Todo,
            owner_member_id: Some("m2".into()),
            claimed_by_member_id: None,
            claim_token: None,
            depends_on: vec![],
            gate_status: "pending".into(),
            gate_reason: None,
            evidence: vec![],
            source_run_id: None,
            order_index: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        // ready, unowned, dep a is done → claimable
        AgentTeamTask {
            id: "c".into(),
            team_id: "t".into(),
            title: "c".into(),
            objective: None,
            status: AgentTeamTaskStatus::Todo,
            owner_member_id: None,
            claimed_by_member_id: None,
            claim_token: None,
            depends_on: vec!["a".into()],
            gate_status: "pending".into(),
            gate_reason: None,
            evidence: vec![],
            source_run_id: None,
            order_index: 2,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
    ];
    let picked = pick_claimable(&tasks, "m1").expect("c is claimable");
    assert_eq!(picked.id, "c");
}

#[test]
fn deliver_pending_messages_injects_then_watermarks() {
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", None);
    // Direct to m1, a broadcast, and one addressed elsewhere.
    super::super::ops::message_member(&config, "team-1", None, Some("m1"), "hello m1", None)
        .unwrap();
    super::super::ops::message_member(&config, "team-1", None, None, "broadcast", None).unwrap();
    seed_member(&config, "team-1", "m2", None);
    super::super::ops::message_member(&config, "team-1", None, Some("m2"), "for m2", None).unwrap();

    let first = deliver_pending_messages(&config, "team-1", "m1").unwrap();
    assert_eq!(first, vec!["hello m1".to_string(), "broadcast".to_string()]);

    // Second call: watermark advanced → nothing new.
    let second = deliver_pending_messages(&config, "team-1", "m1").unwrap();
    assert!(second.is_empty(), "watermark should suppress redelivery");
}

#[test]
fn deliver_pending_messages_pages_past_first_event_page() {
    // Regression: a single `list_recent_run_events` call returns at most one
    // page (1000) from `sequence ASC`, but the pre-fix delivery read one
    // unbounded page (capped at 100). A team with more events than the cap
    // would drop every message AND watermark beyond it. Seed well past one page
    // of filler events, then deliver a message landing at a sequence > the cap.
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", None);

    // Push the sequence far past the old 100-row cap with unrelated events.
    for i in 0..150 {
        run_ledger::append_run_event(
            &config,
            run_ledger::RunEventAppend {
                run_id: "team-1".into(),
                event_type: "noise".into(),
                payload: json!({ "i": i }),
            },
        )
        .unwrap();
    }

    // This message is appended at sequence > 150 — unreachable on the first page.
    super::super::ops::message_member(&config, "team-1", None, Some("m1"), "late note", None)
        .unwrap();

    let delivered = deliver_pending_messages(&config, "team-1", "m1").unwrap();
    assert_eq!(
        delivered,
        vec!["late note".to_string()],
        "message beyond the first event page must still be delivered"
    );

    // Watermark (itself recorded beyond the cap) must also be read back so the
    // redelivery guard holds for long-lived teams.
    let again = deliver_pending_messages(&config, "team-1", "m1").unwrap();
    assert!(
        again.is_empty(),
        "watermark past the first page must suppress redelivery"
    );
}

#[test]
fn build_member_prompt_includes_objective_and_messages() {
    let task = AgentTeamTask {
        id: "t1".into(),
        team_id: "team".into(),
        title: "Ship the widget".into(),
        objective: Some("wire it up".into()),
        status: AgentTeamTaskStatus::InProgress,
        owner_member_id: None,
        claimed_by_member_id: Some("m1".into()),
        claim_token: Some("tok".into()),
        depends_on: vec![],
        gate_status: "pending".into(),
        gate_reason: None,
        evidence: vec![],
        source_run_id: None,
        order_index: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let prompt = build_member_prompt(&task, &["go now".to_string()]);
    assert!(prompt.contains("Ship the widget"));
    assert!(prompt.contains("wire it up"));
    assert!(prompt.contains("go now"));
}
