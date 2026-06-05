use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::debug::DumpPromptOptions;
use openhuman_core::openhuman::agent::task_board::{TaskBoardCard, TaskCardStatus};
use openhuman_core::openhuman::agent::task_dispatcher::{dispatch_card, DispatchOutcome};
use openhuman_core::openhuman::agent::triage::{
    apply_decision, TriageAction, TriageDecision, TriageResolutionPath, TriageRun, TriggerEnvelope,
};
use openhuman_core::openhuman::todos::ops::{self, BoardLocation, CardPatch};
use serde_json::json;
use std::path::Path;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct WorkspaceEnvGuard {
    previous: Option<String>,
}

impl WorkspaceEnvGuard {
    fn set(path: &Path) -> Self {
        let previous = std::env::var("OPENHUMAN_WORKSPACE").ok();
        std::env::set_var("OPENHUMAN_WORKSPACE", path);
        Self { previous }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}

fn board_location(workspace_dir: &Path) -> BoardLocation {
    BoardLocation::Thread {
        workspace_dir: workspace_dir.to_path_buf(),
        thread_id: "round23-triage-dispatch".to_string(),
    }
}

fn add_card(location: &BoardLocation, title: &str, status: TaskCardStatus) -> TaskBoardCard {
    ops::add(
        location,
        title,
        CardPatch {
            status: Some(status),
            objective: Some(format!("Objective for {title}")),
            ..Default::default()
        },
    )
    .expect("card should be added")
    .cards
    .into_iter()
    .next()
    .expect("snapshot should include added card")
}

fn card_status(location: &BoardLocation, card_id: &str) -> TaskCardStatus {
    ops::list(location)
        .expect("board should load")
        .cards
        .into_iter()
        .find(|card| card.id == card_id)
        .expect("card should exist")
        .status
}

fn envelope(external_id: &str) -> TriggerEnvelope {
    TriggerEnvelope::from_composio(
        "github",
        "GITHUB_ISSUE_OPENED",
        "round23",
        external_id,
        json!({ "title": "coverage task" }),
    )
}

fn triage_run(action: TriageAction) -> TriageRun {
    TriageRun {
        decision: TriageDecision {
            action,
            target_agent: Some("orchestrator".to_string()),
            prompt: Some("Handle the linked task card".to_string()),
            reason: "round23 coverage".to_string(),
        },
        used_local: false,
        latency_ms: 7,
        resolution_path: TriageResolutionPath::Cloud,
    }
}

#[tokio::test]
async fn drop_and_acknowledge_gate_pending_linked_cards_without_dispatch() {
    let _env_lock = ENV_LOCK.lock().expect("env lock should not be poisoned");
    let workspace = tempfile::tempdir().expect("temp workspace");
    let _env = WorkspaceEnvGuard::set(workspace.path());
    let location = board_location(workspace.path());

    let drop_card = add_card(&location, "drop me", TaskCardStatus::Todo);
    let drop_envelope =
        envelope("round23-drop").with_task_card(drop_card.id.clone(), location.clone());
    apply_decision(triage_run(TriageAction::Drop), &drop_envelope)
        .await
        .expect("drop should only gate the card");
    assert_eq!(
        card_status(&location, &drop_card.id),
        TaskCardStatus::Rejected
    );

    let ack_card = add_card(&location, "ack me", TaskCardStatus::AwaitingApproval);
    let ack_envelope = envelope("round23-ack").with_task_card(ack_card.id.clone(), location);
    apply_decision(triage_run(TriageAction::Acknowledge), &ack_envelope)
        .await
        .expect("acknowledge should only gate the card");
    assert_eq!(
        card_status(&board_location(workspace.path()), &ack_card.id),
        TaskCardStatus::Rejected
    );
}

#[tokio::test]
async fn react_on_linked_todo_card_parks_for_plan_approval() {
    let _env_lock = ENV_LOCK.lock().expect("env lock should not be poisoned");
    let workspace = tempfile::tempdir().expect("temp workspace");
    let _env = WorkspaceEnvGuard::set(workspace.path());
    let _ = init_global(32);
    let location = board_location(workspace.path());
    let card = add_card(&location, "needs plan approval", TaskCardStatus::Todo);
    let linked = envelope("round23-react").with_task_card(card.id.clone(), location.clone());

    apply_decision(triage_run(TriageAction::React), &linked)
        .await
        .expect("linked todo react should park before autonomous execution");

    assert_eq!(
        card_status(&location, &card.id),
        TaskCardStatus::AwaitingApproval
    );
}

#[tokio::test]
async fn dispatcher_rejects_missing_and_stale_non_claimable_cards() {
    let _env_lock = ENV_LOCK.lock().expect("env lock should not be poisoned");
    let workspace = tempfile::tempdir().expect("temp workspace");
    let _env = WorkspaceEnvGuard::set(workspace.path());
    let location = board_location(workspace.path());

    let missing = TaskBoardCard {
        id: "task-missing-round23".to_string(),
        title: "missing".to_string(),
        status: TaskCardStatus::Todo,
        objective: Some("missing objective".to_string()),
        plan: vec![],
        assigned_agent: None,
        allowed_tools: vec![],
        approval_mode: None,
        acceptance_criteria: vec![],
        evidence: vec![],
        notes: None,
        session_thread_id: None,
        blocker: None,
        source_metadata: None,
        order: 0,
        updated_at: String::new(),
    };
    let missing_err = dispatch_card(location.clone(), missing)
        .await
        .expect_err("missing card should not be claimable");
    assert!(missing_err.contains("not found on board"));

    let stale = add_card(&location, "stale card", TaskCardStatus::Todo);
    ops::update_status(&location, &stale.id, TaskCardStatus::Done)
        .expect("card should be advanced before dispatch");
    let stale_err = dispatch_card(location, stale)
        .await
        .expect_err("stale done card should not be claimable");
    assert!(stale_err.contains("claim rejected"));
    assert!(stale_err.contains("done"));
}

#[tokio::test]
async fn dispatch_card_returns_awaiting_approval_before_agent_spawn() {
    let _env_lock = ENV_LOCK.lock().expect("env lock should not be poisoned");
    let workspace = tempfile::tempdir().expect("temp workspace");
    let _env = WorkspaceEnvGuard::set(workspace.path());
    let _ = init_global(32);
    let location = board_location(workspace.path());
    let card = add_card(&location, "park explicitly", TaskCardStatus::Todo);

    let outcome = dispatch_card(location.clone(), card.clone())
        .await
        .expect("todo card should park for approval under default autonomy config");

    assert!(matches!(outcome, DispatchOutcome::AwaitingApproval));
    assert_eq!(
        card_status(&location, &card.id),
        TaskCardStatus::AwaitingApproval
    );
}

#[test]
fn debug_prompt_options_constructor_sets_safe_defaults() {
    let options = DumpPromptOptions::new("integrations_agent");

    assert_eq!(options.agent_id, "integrations_agent");
    assert!(options.toolkit.is_none());
    assert!(options.workspace_dir_override.is_none());
    assert!(options.model_override.is_none());
}
