//! Live-runtime unit tests (#3374 PR4).
//!
//! The worker-driving core ([`super::drive_member`]) is exercised directly with
//! a mock `ChatModel` passed as the explicit parent (mirroring
//! `workflow_runs::engine_tests`), so a teammate runs deterministically without
//! touching the network. [`super::start_member_run`]'s pre-spawn outcome routing
//! (blocked / already-claimed / no-claimable / unknown) is tested through the
//! real entry point; its `Started` path (which spawns a loop building a real
//! `Agent` from config) is covered by the JSON-RPC e2e over the live core stack.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::agent::harness::fork_context::ParentExecutionContext;
use crate::agent_registry::host_definition::AgentDefinitionRegistry;
use crate::config::{AgentConfig, Config};
use tinyagents_orchestration::teams::{SessionTeamLedger, TeamService};
use tinyagents_session::run_ledger::{
    AgentTeamMemberStatus, AgentTeamMemberUpsert, AgentTeamStatus, AgentTeamTaskStatus,
    AgentTeamTaskUpsert, AgentTeamUpsert,
};
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};
use tinytools::Tool;
use tinytools_agent::dialect::ToolCallFormat;

struct BlockingTeamExecutor {
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Arc<tokio::sync::Notify>,
    dropped: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[async_trait]
impl TeamMemberExecutor for BlockingTeamExecutor {
    async fn execute(
        &self,
        _member: &tinyagents_session::run_ledger::AgentTeamMember,
        _task: &tinyagents_session::run_ledger::AgentTeamTask,
    ) -> anyhow::Result<Vec<String>> {
        if let Some(sender) = self.entered.lock().unwrap().take() {
            let _ = sender.send(());
        }
        self.release.notified().await;
        Ok(vec!["worker result".to_string()])
    }
}

impl Drop for BlockingTeamExecutor {
    fn drop(&mut self) {
        if let Some(sender) = self.dropped.lock().unwrap().take() {
            let _ = sender.send(());
        }
    }
}

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
        host_env: Default::default(),
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
    let ledger = crate::storage::sessions::require_ledger(&config.workspace_dir).unwrap();
    TeamService::new(SessionTeamLedger::new(ledger).unwrap())
}

fn seed_team(config: &Config, team_id: &str) {
    crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = AgentTeamUpsert {
            id: team_id.into(),
            parent_thread_id: None,
            lead_agent_id: "lead".into(),
            status: AgentTeamStatus::Active,
            summary: None,
            created_at: None,
            closed_at: None,
        };
        async move { ledger.upsert_agent_team(ledger_arg_0).await }
    })
    .unwrap();
}

fn seed_member(config: &Config, team_id: &str, member_id: &str, agent_id: Option<&str>) {
    crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = AgentTeamMemberUpsert {
            id: member_id.into(),
            team_id: team_id.into(),
            name: member_id.into(),
            agent_id: agent_id.map(str::to_string),
            member_status: AgentTeamMemberStatus::Idle,
            current_task_id: None,
            worker_thread_id: None,
            run_id: None,
            created_at: None,
        };
        async move { ledger.upsert_agent_team_member(ledger_arg_0).await }
    })
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
    crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = AgentTeamTaskUpsert {
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
        };
        async move { ledger.upsert_agent_team_task(ledger_arg_0).await }
    })
    .unwrap();
}

// ── Host executor adapted through the owner service ────────────────────────

#[tokio::test]
async fn owner_admission_receipt_ids_flow_into_completed_member_evidence() {
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
    let provider = Arc::new(CannedModel {
        output: "did the thing".into(),
        fail: false,
    });
    let executor = HostTeamMemberExecutor {
        ctx: crate::core::runtime::DispatchCtx::ambient(),
        config: config.clone(),
        team_id: "team-1".into(),
        member_id: "m1".into(),
        model_override: Some("test-model".into()),
        parent_override: Some(mock_parent(provider)),
    };
    let mut receipt = None;
    let completion = team_service(&config)
        .start_member_with_started("team-1", "task-a", "m1", &executor, false, |started| {
            receipt = Some(started);
        })
        .await
        .expect("owner completes member");
    assert!(matches!(
        completion,
        tinyagents_session::run_ledger::CompletionOutcome::Completed(_)
    ));
    let receipt = receipt.expect("owner returns actual admission receipt");

    let done = crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = ("task-a").to_string();
        async move { ledger.get_agent_team_task(&ledger_arg_0).await }
    })
    .unwrap()
    .unwrap();
    assert_eq!(done.status, AgentTeamTaskStatus::Done);
    assert_eq!(done.gate_status, "passed");
    assert_eq!(done.evidence.len(), 1, "worker output captured as evidence");
    assert!(done.evidence[0].contains(&receipt.run_id));
    assert_eq!(receipt.task.id, "task-a");
    assert_eq!(
        receipt.member.run_id.as_deref(),
        Some(receipt.run_id.as_str())
    );
    assert!(receipt.member.worker_thread_id.is_some());

    let member = crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = ("m1").to_string();
        async move { ledger.get_agent_team_member(&ledger_arg_0).await }
    })
    .unwrap()
    .unwrap();
    assert_eq!(member.member_status, AgentTeamMemberStatus::Idle);
    assert_eq!(member.current_task_id, None);
}

/// Covers `run_member_loop` — the `with_root_parent` wrapper around
/// `drive_member`. With a mock parent passed explicitly, `with_root_parent`
/// uses it (rather than building a real root), so the member drives to
/// execution under the canned provider and returns owner-ready evidence.
#[tokio::test]
async fn run_member_loop_drives_member_under_an_explicit_parent() {
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
    let task = crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = ("task-a").to_string();
        async move { ledger.get_agent_team_task(&ledger_arg_0).await }
    })
    .unwrap()
    .unwrap();

    let provider = Arc::new(CannedModel {
        output: "did the thing".into(),
        fail: false,
    });
    let evidence = run_member_loop(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "team-1",
        "m1",
        "task_manager_agent",
        task,
        "teamrun-y",
        "teamworker-y",
        Some("test-model".into()),
        Some(mock_parent(provider)),
    )
    .await
    .expect("run_member_loop returns evidence through with_root_parent");
    assert_eq!(evidence.len(), 1);
    assert!(evidence[0].contains("teamrun-y"));
}

#[tokio::test(start_paused = true)]
async fn owner_releases_task_when_host_worker_fails() {
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
    let provider = Arc::new(CannedModel {
        output: String::new(),
        fail: true,
    });
    let executor = HostTeamMemberExecutor {
        ctx: crate::core::runtime::DispatchCtx::ambient(),
        config: config.clone(),
        team_id: "team-1".into(),
        member_id: "m1".into(),
        model_override: Some("test-model".into()),
        parent_override: Some(mock_parent(provider)),
    };
    assert!(team_service(&config)
        .start_member_with_started("team-1", "task-a", "m1", &executor, false, |_| {})
        .await
        .is_err());

    // Task released back to todo, claim cleared → reclaimable.
    let released = crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = ("task-a").to_string();
        async move { ledger.get_agent_team_task(&ledger_arg_0).await }
    })
    .unwrap()
    .unwrap();
    assert_eq!(released.status, AgentTeamTaskStatus::Todo);
    assert_eq!(released.claimed_by_member_id, None);

    let member = crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = ("m1").to_string();
        async move { ledger.get_agent_team_member(&ledger_arg_0).await }
    })
    .unwrap()
    .unwrap();
    assert_eq!(member.member_status, AgentTeamMemberStatus::Idle);
}

#[tokio::test]
async fn start_rpc_returns_owner_receipt_while_member_executor_is_blocked() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let (_dir, config) = test_config();
    seed_team(&config, "team-rpc");
    seed_member(
        &config,
        "team-rpc",
        "member-rpc",
        Some("task_manager_agent"),
    );
    seed_task(
        &config,
        "team-rpc",
        "task-rpc",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new(tokio::sync::Notify::new());
    let executor = BlockingTeamExecutor {
        entered: std::sync::Mutex::new(Some(entered_tx)),
        release: Arc::clone(&release),
        dropped: std::sync::Mutex::new(Some(dropped_tx)),
    };
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let task_config = config.clone();
    let start = tokio::spawn(async move {
        start_member_with_executor(
            &ctx,
            &task_config,
            "team-rpc",
            "member-rpc",
            crate::storage::sessions::with_ledger(&task_config.workspace_dir, |ledger| {
                let id = "task-rpc".to_string();
                async move { ledger.get_agent_team_task(&id).await }
            })
            .unwrap()
            .unwrap(),
            executor,
        )
        .await
    });

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), start)
        .await
        .expect("start RPC waits only for owner admission")
        .expect("start task joins")
        .expect("owner admission succeeds");
    let StartMemberOutcome::Started { run_id, task } = outcome else {
        panic!("expected started receipt, got {outcome:?}");
    };
    assert_eq!(task.id, "task-rpc");
    tokio::time::timeout(std::time::Duration::from_secs(5), entered_rx)
        .await
        .expect("executor started")
        .expect("executor signal delivered");

    let active = crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let id = "member-rpc".to_string();
        async move { ledger.get_agent_team_member(&id).await }
    })
    .unwrap()
    .unwrap();
    assert_eq!(active.member_status, AgentTeamMemberStatus::Active);
    assert_eq!(active.run_id.as_deref(), Some(run_id.as_str()));
    assert!(active.worker_thread_id.is_some());

    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), dropped_rx)
        .await
        .expect("owner finishes after executor release")
        .expect("executor dropped after owner completion");
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
    let outcome = start_member_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "team-1",
        "m1",
        Some("task-b"),
        None,
    )
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
    crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = ("team-1").to_string();
        let ledger_arg_1 = ("task-a").to_string();
        let ledger_arg_2 = ("m2").to_string();
        let ledger_arg_3 = ("tok").to_string();
        async move {
            ledger
                .claim_agent_team_task(&ledger_arg_0, &ledger_arg_1, &ledger_arg_2, &ledger_arg_3)
                .await
        }
    })
    .unwrap();

    let outcome = start_member_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "team-1",
        "m1",
        Some("task-a"),
        None,
    )
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
    let none = start_member_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "team-1",
        "m1",
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(none, StartMemberOutcome::NoClaimableTask);

    // Explicit unknown task id → UnknownTask.
    let unknown = start_member_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "team-1",
        "m1",
        Some("ghost"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(unknown, StartMemberOutcome::UnknownTask);

    // Unknown member → typed error.
    let err = start_member_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "team-1",
        "ghost",
        None,
        None,
    )
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
    crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = AgentTeamMemberUpsert {
            id: "m1".into(),
            team_id: "team-1".into(),
            name: "m1".into(),
            agent_id: Some("researcher".into()),
            member_status: AgentTeamMemberStatus::Active,
            current_task_id: Some("t-running".into()),
            worker_thread_id: Some("run-running".into()),
            run_id: Some("run-running".into()),
            created_at: None,
        };
        async move { ledger.upsert_agent_team_member(ledger_arg_0).await }
    })
    .unwrap();
    seed_task(
        &config,
        "team-1",
        "t-free",
        AgentTeamTaskStatus::Todo,
        None,
        vec![],
    );

    let outcome = start_member_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "team-1",
        "m1",
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(outcome, StartMemberOutcome::AlreadyActive);

    // No claim happened — the free task is untouched and the member still points
    // at its original run (no clobbered pointer).
    let task = crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = ("t-free").to_string();
        async move { ledger.get_agent_team_task(&ledger_arg_0).await }
    })
    .unwrap()
    .expect("task exists");
    assert_eq!(task.status, AgentTeamTaskStatus::Todo);
    assert!(task.claimed_by_member_id.is_none());
    let member = crate::storage::sessions::with_ledger(&config.workspace_dir, |ledger| {
        let ledger_arg_0 = ("m1").to_string();
        async move { ledger.get_agent_team_member(&ledger_arg_0).await }
    })
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
