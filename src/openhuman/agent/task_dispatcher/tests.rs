//! Unit tests for the task dispatcher sub-modules.

use serde_json::json;

use crate::openhuman::agent::task_board::{TaskApprovalMode, TaskBoardCard, TaskCardStatus};
use crate::openhuman::todos::ops::{self, BoardLocation, CardPatch};

use super::executor::{truncate_chars, write_back, EVIDENCE_MAX_CHARS};
use super::poller::{pick_next_todo, requires_plan_approval};
use super::prompt::{build_progress_instruction, build_task_prompt};
use super::registry::{register_active_run, take_active_run};
use super::types::ActiveRun;

#[tokio::test]
async fn active_run_registry_take_is_once() {
    // Race-safety: the completing run and a concurrent cancel both call
    // `take_active_run`; exactly one gets `Some` (and owns the write-back).
    let (tx, _rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(async { std::future::pending::<()>().await });
    let key = "task-cancel-registry-test";
    register_active_run(
        key.to_string(),
        ActiveRun {
            abort: handle.abort_handle(),
            hb_cancel: tx,
            location: BoardLocation::Scratch,
            card_id: "c1".to_string(),
            run_id: "r1".to_string(),
        },
    );
    assert!(take_active_run(key).is_some(), "first take owns the run");
    assert!(
        take_active_run(key).is_none(),
        "second take gets nothing — write-back happens exactly once"
    );
    handle.abort();
}

fn card(objective: Option<&str>) -> TaskBoardCard {
    TaskBoardCard {
        id: "task-1".into(),
        title: "[GitHub] Fix login bug".into(),
        status: TaskCardStatus::Todo,
        objective: objective.map(str::to_string),
        plan: vec![],
        assigned_agent: None,
        allowed_tools: vec![],
        approval_mode: None,
        acceptance_criteria: vec![],
        evidence: vec![],
        notes: None,
        blocker: None,
        session_thread_id: None,
        source_metadata: None,
        order: 0,
        updated_at: String::new(),
    }
}

#[test]
fn prompt_uses_objective_then_falls_back_to_title() {
    let p = build_task_prompt(&card(Some("Fix the login bug")));
    assert!(p.contains("Fix the login bug"));
    assert!(!p.contains("[GitHub]"));

    let p2 = build_task_prompt(&card(None));
    assert!(p2.contains("[GitHub] Fix login bug"));
}

#[test]
fn prompt_includes_plan_and_acceptance_criteria() {
    let mut c = card(Some("Do it"));
    c.plan = vec!["step one".into(), "step two".into()];
    c.acceptance_criteria = vec!["tests pass".into()];
    let p = build_task_prompt(&c);
    assert!(p.contains("Plan:"));
    assert!(p.contains("1. step one"));
    assert!(p.contains("2. step two"));
    assert!(p.contains("Acceptance criteria"));
    assert!(p.contains("- tests pass"));
}

#[test]
fn prompt_points_at_source_and_memory_when_metadata_present() {
    let mut c = card(Some("Resolve issue"));
    c.source_metadata = Some(json!({
        "provider": "github",
        "repo": "octo/repo",
        "external_id": "123",
        "url": "https://github.com/octo/repo/issues/123",
    }));
    let p = build_task_prompt(&c);
    assert!(p.contains("github octo/repo#123"));
    assert!(p.contains("memory_recall"));
    assert!(p.contains("https://github.com/octo/repo/issues/123"));
}

#[test]
fn prompt_omits_source_block_without_metadata() {
    let p = build_task_prompt(&card(Some("Do it")));
    assert!(!p.contains("memory_recall"));
    assert!(!p.contains("record the outcome on the upstream source"));
}

#[test]
fn prompt_includes_external_writeback_when_addressable() {
    let mut c = card(Some("Resolve issue"));
    c.source_metadata = Some(json!({
        "provider": "github",
        "repo": "octo/repo",
        "external_id": "123",
    }));
    let p = build_task_prompt(&c);
    assert!(p.contains("record the outcome on the upstream source"));
    assert!(p.contains("close/resolve the item"));
}

#[test]
fn prompt_omits_writeback_when_not_addressable() {
    // Urgency-only metadata (no provider/external_id) can't address an
    // upstream item, so no write-back instruction.
    let mut c = card(Some("Do it"));
    c.source_metadata = Some(json!({ "urgency": 0.5 }));
    let p = build_task_prompt(&c);
    assert!(!p.contains("record the outcome on the upstream source"));
}

#[test]
fn truncate_caps_long_strings() {
    let s = "x".repeat(5_000);
    let out = truncate_chars(&s, EVIDENCE_MAX_CHARS);
    assert!(out.chars().count() <= EVIDENCE_MAX_CHARS);
    assert!(out.ends_with('…'));
}

fn card_with(id: &str, status: TaskCardStatus, urgency: Option<f64>, order: u32) -> TaskBoardCard {
    let mut c = card(Some("obj"));
    c.id = id.into();
    c.status = status;
    c.order = order;
    c.source_metadata = urgency.map(|u| json!({ "urgency": u }));
    c
}

#[test]
fn poller_picks_highest_urgency_todo_skipping_other_statuses() {
    let cards = vec![
        card_with("a", TaskCardStatus::Todo, Some(0.3), 0),
        card_with("b", TaskCardStatus::Done, Some(0.99), 1),
        card_with("c", TaskCardStatus::Todo, Some(0.8), 2),
        card_with("d", TaskCardStatus::Todo, None, 3),
    ];
    let picked = pick_next_todo(&cards, false).expect("a todo card is available");
    assert_eq!(
        picked.id, "c",
        "highest-urgency todo wins, done card ignored"
    );
}

#[test]
fn poller_breaks_urgency_ties_toward_lower_order() {
    let cards = vec![
        card_with("late", TaskCardStatus::Todo, Some(0.5), 5),
        card_with("early", TaskCardStatus::Todo, Some(0.5), 2),
    ];
    assert_eq!(pick_next_todo(&cards, false).unwrap().id, "early");
}

#[test]
fn poller_returns_none_when_no_todo_cards() {
    let cards = vec![card_with("a", TaskCardStatus::Done, Some(0.9), 0)];
    assert!(pick_next_todo(&cards, false).is_none());
}

#[test]
fn poller_dispatches_ready_cards_and_skips_approval_states() {
    // Approved `ready` cards are dispatchable; `awaiting_approval` and
    // `rejected` are not.
    let cards = vec![
        card_with("await", TaskCardStatus::AwaitingApproval, Some(0.99), 0),
        card_with("rej", TaskCardStatus::Rejected, Some(0.95), 1),
        card_with("ready", TaskCardStatus::Ready, Some(0.5), 2),
    ];
    assert_eq!(pick_next_todo(&cards, false).unwrap().id, "ready");
}

#[test]
fn poller_prefers_higher_urgency_across_todo_and_ready() {
    let cards = vec![
        card_with("ready-low", TaskCardStatus::Ready, Some(0.3), 0),
        card_with("todo-high", TaskCardStatus::Todo, Some(0.9), 1),
    ];
    assert_eq!(pick_next_todo(&cards, false).unwrap().id, "todo-high");
}

#[test]
fn poller_agent_only_skips_unassigned_cards() {
    // On the user-tasks board we run only agent-assigned cards. A human's
    // manual todo (no assigned_agent) must be skipped even at high urgency.
    let mut human = card_with("human", TaskCardStatus::Todo, Some(0.99), 0);
    human.assigned_agent = None;
    let mut agent = card_with("agent", TaskCardStatus::Todo, Some(0.20), 1);
    agent.assigned_agent = Some("orchestrator".into());
    let cards = vec![human, agent];

    // Agent-only: the lower-urgency assigned card wins; the human card is invisible.
    assert_eq!(pick_next_todo(&cards, true).unwrap().id, "agent");
    // Unfiltered (task-sources behaviour): highest urgency wins regardless.
    assert_eq!(pick_next_todo(&cards, false).unwrap().id, "human");
}

#[test]
fn poller_agent_only_returns_none_when_all_unassigned() {
    let mut a = card_with("a", TaskCardStatus::Todo, Some(0.9), 0);
    a.assigned_agent = None;
    let mut b = card_with("b", TaskCardStatus::Todo, Some(0.5), 1);
    b.assigned_agent = Some("   ".into()); // blank handle is not "assigned"
    let cards = vec![a, b];
    assert!(pick_next_todo(&cards, true).is_none());
}

#[test]
fn approval_gate_respects_global_and_per_card_optout() {
    // Unset → follow the global default.
    assert!(!requires_plan_approval(false, None));
    assert!(requires_plan_approval(true, None));
    // Explicit `Required` always parks — even when the global default is off
    // (the interactive plan-review gate stamps `Required`).
    assert!(requires_plan_approval(
        false,
        Some(&TaskApprovalMode::Required)
    ));
    assert!(requires_plan_approval(
        true,
        Some(&TaskApprovalMode::Required)
    ));
    // Explicit `NotRequired` never parks, regardless of the global default.
    assert!(!requires_plan_approval(
        true,
        Some(&TaskApprovalMode::NotRequired)
    ));
    assert!(!requires_plan_approval(
        false,
        Some(&TaskApprovalMode::NotRequired)
    ));
}

#[test]
fn progress_instruction_names_card_thread_and_tool() {
    let s = build_progress_instruction("task-42", "user-tasks");
    assert!(s.contains("task-42"));
    assert!(s.contains("user-tasks"));
    assert!(s.contains("update_task"));
    // It must instruct the agent to self-block (status: blocked + blocker)
    // when it needs the user, so write_back can preserve that state.
    assert!(s.contains("status: blocked"));
    assert!(s.contains("blocker"));
}

#[test]
fn resolver_defaults_to_orchestrator_for_unset_or_orchestrator_handle() {
    use super::executor::resolve_executor;
    let dir = tempfile::tempdir().unwrap();
    for handle in [None, Some(""), Some("   "), Some("orchestrator")] {
        let r = resolve_executor(dir.path(), handle);
        assert_eq!(r.agent_id, "orchestrator");
        assert_eq!(r.label, "default");
        assert!(r.prompt_suffix.is_none());
    }
}

#[test]
fn resolver_uses_personality_branch_for_builtin_profile() {
    use super::executor::resolve_executor;
    // `load_profiles` returns built-in profiles for any empty workspace, so
    // the personality branch is reachable with no fixture file. "research"
    // is a built-in profile backed by the "researcher" agent.
    let dir = tempfile::tempdir().unwrap();
    let r = resolve_executor(dir.path(), Some("research"));
    assert_eq!(r.label, "personality:research");
    assert_eq!(r.agent_id, "researcher");
    let suffix = r.prompt_suffix.expect("personality preamble present");
    assert!(suffix.contains("acting as the personality `research`"));
}

#[test]
fn resolver_degrades_to_default_for_unresolved_handle() {
    use super::executor::resolve_executor;
    let dir = tempfile::tempdir().unwrap();
    let r = resolve_executor(dir.path(), Some("no-such-executor-xyz"));
    assert_eq!(r.agent_id, "orchestrator");
    assert_eq!(r.label, "default-fallback");
    assert!(r.prompt_suffix.is_none());
}

fn board_loc(dir: &std::path::Path) -> BoardLocation {
    BoardLocation::Thread {
        workspace_dir: dir.to_path_buf(),
        thread_id: "t1".to_string(),
    }
}

#[test]
fn write_back_marks_done_with_evidence_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let loc = board_loc(dir.path());
    let id = ops::add(&loc, "do the thing", CardPatch::default())
        .unwrap()
        .cards[0]
        .id
        .clone();
    ops::update_status(&loc, &id, TaskCardStatus::InProgress).unwrap();

    write_back(
        &loc,
        &id,
        "run-1",
        Ok("completed: opened PR #5".to_string()),
    );

    let card = ops::list(&loc)
        .unwrap()
        .cards
        .into_iter()
        .find(|c| c.id == id)
        .unwrap();
    assert_eq!(card.status, TaskCardStatus::Done);
    assert!(card.evidence.iter().any(|e| e.contains("opened PR #5")));
}

#[test]
fn write_back_preserves_agent_set_blocked_on_clean_run() {
    // The run marked its own card `blocked` (needs user input) via
    // update_task, then ended cleanly. write_back must NOT force it to
    // `done` — the task stays blocked, with the agent's blocker intact,
    // awaiting the user.
    let dir = tempfile::tempdir().unwrap();
    let loc = board_loc(dir.path());
    let id = ops::add(&loc, "update alan", CardPatch::default())
        .unwrap()
        .cards[0]
        .id
        .clone();
    ops::update_status(&loc, &id, TaskCardStatus::InProgress).unwrap();
    // Agent self-blocks mid-run, as build_progress_instruction asks it to.
    ops::edit(
        &loc,
        &id,
        CardPatch {
            status: Some(TaskCardStatus::Blocked),
            blocker: Some("Slack isn't connected — confirm how to reach Alan".to_string()),
            ..Default::default()
        },
    )
    .unwrap();

    // Run returns Ok (the turn finished) — but the card is self-blocked.
    write_back(
        &loc,
        &id,
        "run-2",
        Ok("I checked GitHub and memory…".to_string()),
    );

    let card = ops::list(&loc)
        .unwrap()
        .cards
        .into_iter()
        .find(|c| c.id == id)
        .unwrap();
    assert_eq!(
        card.status,
        TaskCardStatus::Blocked,
        "a clean run over a self-blocked card must stay blocked, not auto-done"
    );
    assert_eq!(
        card.blocker.as_deref(),
        Some("Slack isn't connected — confirm how to reach Alan"),
        "the agent's blocker reason is preserved"
    );
}

#[test]
fn write_back_marks_blocked_with_reason_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let loc = board_loc(dir.path());
    let id = ops::add(&loc, "do the thing", CardPatch::default())
        .unwrap()
        .cards[0]
        .id
        .clone();
    ops::update_status(&loc, &id, TaskCardStatus::InProgress).unwrap();

    write_back(&loc, &id, "run-1", Err("agent build failed".to_string()));

    let card = ops::list(&loc)
        .unwrap()
        .cards
        .into_iter()
        .find(|c| c.id == id)
        .unwrap();
    assert_eq!(card.status, TaskCardStatus::Blocked);
    assert!(card
        .blocker
        .as_deref()
        .unwrap_or_default()
        .contains("agent build failed"));
}
