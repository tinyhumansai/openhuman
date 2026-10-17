use super::*;
use tinyagents_session::turn_state::ToolTimelineEntry;

fn entry(value: serde_json::Value) -> ToolTimelineEntry {
    serde_json::from_value(value).expect("tool timeline entry")
}

fn snapshot() -> TurnState {
    let mut state = TurnState::started("thread-1", "req-1", 50, "2026-10-13T21:28:58Z");
    state.tool_timeline = vec![
        entry(serde_json::json!({
            "id": "c1", "name": "file_write", "round": 1, "status": "success",
            "displayName": "Write file", "detail": "crates/tiny-artifacts/Cargo.toml"
        })),
        entry(serde_json::json!({
            "id": "c2", "name": "shell", "round": 2, "status": "error",
            "detail": "cargo build",
            "failure": {
                "class": "CommandFailed", "category": "Recoverable", "recoverable": true,
                "causePlain": "the build failed", "nextAction": "fix the error"
            }
        })),
        entry(serde_json::json!({
            "id": "c3", "name": "shell", "round": 3, "status": "running",
            "detail": "git worktree add"
        })),
    ];
    state.transcript = vec![
        TranscriptItem::Narration {
            round: 1,
            seq: 0,
            text: "Creating the crate first.".into(),
        },
        TranscriptItem::Narration {
            round: 3,
            seq: 4,
            text: "Now setting up the worktree.".into(),
        },
    ];
    state
}

fn stopped(cause: StopCause, stuck: StuckPoint) -> StoppedTurn<'static> {
    StoppedTurn {
        thread_id: "thread-1",
        request_id: "req-1",
        cause,
        stuck,
        config: None,
        locale: None,
        ctx: crate::core::runtime::DispatchCtx::host_default(),
    }
}

#[test]
fn evidence_lists_every_step_with_its_outcome_and_the_last_narration() {
    let evidence = Evidence::from_snapshot(Some(&snapshot()));
    assert_eq!(
        evidence.steps,
        vec![
            "done: Write file (crates/tiny-artifacts/Cargo.toml)".to_string(),
            "failed: shell (cargo build) — the build failed".to_string(),
            "still running: shell (git worktree add)".to_string(),
        ]
    );
    assert_eq!(
        evidence.narration.as_deref(),
        Some("Now setting up the worktree.")
    );
}

#[test]
fn evidence_keeps_only_the_latest_steps() {
    let mut state = snapshot();
    let row = state.tool_timeline[0].clone();
    state.tool_timeline = vec![row; MAX_STEPS + 5];
    assert_eq!(Evidence::from_snapshot(Some(&state)).steps.len(), MAX_STEPS);
}

#[test]
fn no_snapshot_is_no_evidence() {
    assert_eq!(Evidence::from_snapshot(None), Evidence::default());
}

#[test]
fn plain_reply_says_why_where_and_what_got_done() {
    let turn = stopped(
        StopCause::Idle(Duration::from_secs(900)),
        StuckPoint::BetweenSteps {
            after: "the `shell` tool finished".into(),
        },
    );
    let reply = plain_reply(&turn, &Evidence::from_snapshot(Some(&snapshot())));
    assert!(
        reply.starts_with(
            "I stopped this turn because nothing happened for 15 minutes. It got stuck \
             between steps: the `shell` tool finished, and the agent's next step never \
             started."
        ),
        "{reply}"
    );
    assert!(reply.contains("- failed: shell (cargo build) — the build failed"));
    assert!(reply.ends_with("Say \"continue\" and I'll pick up from here."));
}

#[test]
fn plain_reply_with_nothing_recorded() {
    let turn = stopped(
        StopCause::Ceiling(Duration::from_secs(3_600)),
        StuckPoint::AfterRun,
    );
    let reply = plain_reply(&turn, &Evidence::default());
    assert!(
        reply.contains("it hit its time limit of 60 minutes"),
        "{reply}"
    );
    assert!(reply.contains("while its reply was being saved"), "{reply}");
    assert!(reply.contains("No steps had been recorded yet."), "{reply}");
}

#[test]
fn durations_read_naturally() {
    assert_eq!(minutes(Duration::from_secs(1)), "1 second");
    assert_eq!(minutes(Duration::from_secs(45)), "45 seconds");
    assert_eq!(minutes(Duration::from_secs(60)), "1 minute");
    assert_eq!(minutes(Duration::from_secs(900)), "15 minutes");
}

#[test]
fn the_close_request_carries_the_cause_the_stuck_point_and_the_evidence() {
    let turn = stopped(
        StopCause::Idle(Duration::from_secs(900)),
        StuckPoint::BeforeRun,
    );
    let request = close_request(&turn, &Evidence::from_snapshot(Some(&snapshot())));
    let prompt = format!("{:?}", request.messages);
    assert!(prompt.contains("nothing happened for 15 minutes"));
    assert!(prompt.contains("before the agent started"));
    assert!(prompt.contains("still running: shell (git worktree add)"));
    assert!(request.tools.is_empty(), "the close call has no tools");
}

/// A turn stopped before its run never handed the request to the agent, so
/// "continue" would continue nothing: the user is asked to send it again.
#[test]
fn a_request_the_agent_never_saw_is_asked_for_again() {
    let turn = stopped(
        StopCause::Idle(Duration::from_secs(900)),
        StuckPoint::BeforeRun,
    );
    let reply = plain_reply(&turn, &Evidence::default());
    assert!(reply.ends_with("Please send it again."), "{reply}");
    assert!(!reply.contains("continue"), "{reply}");
    let prompt = format!("{:?}", close_request(&turn, &Evidence::default()).messages);
    assert!(prompt.contains("never received the user's request"));
}

#[test]
fn the_close_request_carries_the_turn_reply_language() {
    let mut turn = stopped(
        StopCause::Idle(Duration::from_secs(900)),
        StuckPoint::AfterRun,
    );
    turn.locale = Some("de".into());
    let prompt = format!("{:?}", close_request(&turn, &Evidence::default()).messages);
    assert!(prompt.contains("German"), "{prompt}");
    turn.locale = None;
    let prompt = format!("{:?}", close_request(&turn, &Evidence::default()).messages);
    assert!(!prompt.contains("German"));
}
