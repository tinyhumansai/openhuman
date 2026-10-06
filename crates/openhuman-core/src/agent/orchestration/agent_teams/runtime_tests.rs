//! Live-runtime unit tests (#3374 PR4).
//!
//! The worker-driving core ([`super::drive_member`]) is exercised directly with
//! a mock `ChatModel` installed via [`with_parent_context`] (mirroring
//! `workflow_runs::engine_tests`), so a teammate runs deterministically without
//! touching the network. [`super::start_member_run`]'s pre-spawn outcome routing
//! (blocked / already-claimed / no-claimable / unknown) is tested through the
//! real entry point; its `Started` path (which spawns a loop building a real
//! `Agent` from config) is covered by the JSON-RPC e2e over the live core stack.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::agent::prompts::ToolCallFormat;
use crate::config::{AgentConfig, Config};
use tinyagents_orchestration::teams::{SessionTeamLedger, TeamService};
use tinyagents_session::run_ledger::{
    self, AgentTeamMemberStatus, AgentTeamMemberUpsert, AgentTeamStatus, AgentTeamTaskStatus,
    AgentTeamTaskUpsert, AgentTeamUpsert,
};
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};
use tinytools::Tool;

// ── Mocks (mirror workflow_runs::engine_tests) ──────────────────────────────

fn text_response(text: impl Into<String>) -> ModelResponse {
    ModelResponse::assistant(text)
}

/// Mock model that answers every child with a fixed completion, or fails.
#[derive(Clone)]
struct CannedModel {
    output: String,
    fail: bool,
}

#[async_trait]
impl ChatModel<()> for CannedModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        if self.fail {
            return Err(tinyinference_llm::Error::Model(
                "mock model forced failure".to_string(),
            ));
        }
        Ok(text_response(self.output.clone()))
    }
}

fn mock_parent(model: Arc<dyn ChatModel<()>>) -> ParentExecutionContext {
    ParentExecutionContext {
        runtime_config: None,
        workspace_descriptor: None,
        agent_definition_id: "agent_team_runtime".to_string(),
        allowed_subagent_ids: HashSet::new(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(Vec::<Box<dyn Tool>>::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".to_string(),
        temperature: 0.0,
        workspace_dir: std::env::temp_dir(),
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

fn team_service(config: &Config) -> TeamService<SessionTeamLedger> {
    TeamService::new(SessionTeamLedger::new(config.workspace_dir.clone()))
}

fn seed_team(config: &Config, team_id: &str) {
    run_ledger::upsert_agent_team(
        &config.workspace_dir,
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
        &config.workspace_dir,
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
        &config.workspace_dir,
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
    seed_member(&config, "team-1", "m1", Some("task_manager_agent"));
    seed_task(
        &config,
        "team-1",
        "task-a",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    // Claim + mark running, mirroring what start_member_run does pre-spawn.
    run_ledger::claim_agent_team_task(&config.workspace_dir, "team-1", "task-a", "m1", "teamrun-x")
        .unwrap();
    run_ledger::mark_agent_team_member_running(
        &config.workspace_dir,
        "team-1",
        "m1",
        "task-a",
        "teamrun-x",
        "teamrun-x",
    )
    .unwrap();
    let task = run_ledger::get_agent_team_task(&config.workspace_dir, "task-a")
        .unwrap()
        .unwrap();

    let provider = Arc::new(CannedModel {
        output: "did the thing".into(),
        fail: false,
    });
    with_parent_context(mock_parent(provider), async {
        drive_member(
            &config,
            "team-1",
            "m1",
            "task_manager_agent",
            &task,
            "teamrun-x",
            Some("test-model".into()),
        )
        .await
    })
    .await
    .expect("drive_member ok");

    let done = run_ledger::get_agent_team_task(&config.workspace_dir, "task-a")
        .unwrap()
        .unwrap();
    assert_eq!(done.status, AgentTeamTaskStatus::Done);
    assert_eq!(done.gate_status, "passed");
    assert_eq!(done.evidence.len(), 1, "worker output captured as evidence");
    assert!(done.evidence[0].contains("teamrun-x"));

    let member = run_ledger::get_agent_team_member(&config.workspace_dir, "m1")
        .unwrap()
        .unwrap();
    assert_eq!(member.member_status, AgentTeamMemberStatus::Idle);
    assert_eq!(member.current_task_id, None);
}

/// Covers `run_member_loop` — the `with_root_parent` wrapper around
/// `drive_member`. With a mock parent already installed, `with_root_parent`
/// reuses it (rather than building a real root), so the member drives to
/// completion under the canned provider. Same setup as the `drive_member`
/// happy-path test, but through the wrapper the live runtime spawns.
#[tokio::test]
async fn run_member_loop_drives_member_under_ambient_parent() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("task_manager_agent"));
    seed_task(
        &config,
        "team-1",
        "task-a",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    run_ledger::claim_agent_team_task(&config.workspace_dir, "team-1", "task-a", "m1", "teamrun-y")
        .unwrap();
    run_ledger::mark_agent_team_member_running(
        &config.workspace_dir,
        "team-1",
        "m1",
        "task-a",
        "teamrun-y",
        "teamrun-y",
    )
    .unwrap();
    let task = run_ledger::get_agent_team_task(&config.workspace_dir, "task-a")
        .unwrap()
        .unwrap();

    let provider = Arc::new(CannedModel {
        output: "did the thing".into(),
        fail: false,
    });
    with_parent_context(mock_parent(provider), async {
        run_member_loop(
            &config,
            "team-1",
            "m1",
            "task_manager_agent",
            task,
            "teamrun-y",
            Some("test-model".into()),
        )
        .await
    })
    .await;

    let done = run_ledger::get_agent_team_task(&config.workspace_dir, "task-a")
        .unwrap()
        .unwrap();
    assert_eq!(
        done.status,
        AgentTeamTaskStatus::Done,
        "run_member_loop drove the member to completion through with_root_parent"
    );
}

#[tokio::test]
async fn drive_member_releases_task_when_worker_fails() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("task_manager_agent"));
    seed_task(
        &config,
        "team-1",
        "task-a",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    run_ledger::claim_agent_team_task(&config.workspace_dir, "team-1", "task-a", "m1", "teamrun-x")
        .unwrap();
    run_ledger::mark_agent_team_member_running(
        &config.workspace_dir,
        "team-1",
        "m1",
        "task-a",
        "teamrun-x",
        "teamrun-x",
    )
    .unwrap();
    let task = run_ledger::get_agent_team_task(&config.workspace_dir, "task-a")
        .unwrap()
        .unwrap();

    let provider = Arc::new(CannedModel {
        output: String::new(),
        fail: true,
    });
    with_parent_context(mock_parent(provider), async {
        drive_member(
            &config,
            "team-1",
            "m1",
            "task_manager_agent",
            &task,
            "teamrun-x",
            Some("test-model".into()),
        )
        .await
    })
    .await
    .expect("drive_member handles worker failure without erroring");

    // Task released back to todo, claim cleared → reclaimable.
    let released = run_ledger::get_agent_team_task(&config.workspace_dir, "task-a")
        .unwrap()
        .unwrap();
    assert_eq!(released.status, AgentTeamTaskStatus::Todo);
    assert_eq!(released.claimed_by_member_id, None);

    let member = run_ledger::get_agent_team_member(&config.workspace_dir, "m1")
        .unwrap()
        .unwrap();
    assert_eq!(member.member_status, AgentTeamMemberStatus::Idle);
}

// ── start_member_run pre-spawn outcome routing ──────────────────────────────

#[tokio::test]
async fn start_member_run_blocks_on_unmet_dependency() {
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("task_manager_agent"));
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
    seed_member(&config, "team-1", "m1", Some("task_manager_agent"));
    seed_member(&config, "team-1", "m2", Some("task_manager_agent"));
    seed_task(
        &config,
        "team-1",
        "task-a",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    // m2 already holds task-a.
    run_ledger::claim_agent_team_task(&config.workspace_dir, "team-1", "task-a", "m2", "tok")
        .unwrap();

    let outcome = start_member_run(&config, "team-1", "m1", Some("task-a"), None)
        .await
        .unwrap();
    assert_eq!(outcome, StartMemberOutcome::AlreadyClaimed);
}

#[tokio::test]
async fn start_member_run_no_claimable_and_unknown_task() {
    let (_dir, config) = test_config();
    seed_team(&config, "team-1");
    seed_member(&config, "team-1", "m1", Some("task_manager_agent"));
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
        &config.workspace_dir,
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
    let task = run_ledger::get_agent_team_task(&config.workspace_dir, "t-free")
        .unwrap()
        .expect("task exists");
    assert_eq!(task.status, AgentTeamTaskStatus::Todo);
    assert!(task.claimed_by_member_id.is_none());
    let member = run_ledger::get_agent_team_member(&config.workspace_dir, "m1")
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
    let picked =
        tinyagents_orchestration::teams::claimable_task(&tasks, "m1").expect("c is claimable");
    assert_eq!(picked.id, "c");
}
