use super::*;
use crate::openhuman::agent::task_board::{TaskBoardCard, TaskCardStatus};

fn card(title: &str) -> TaskBoardCard {
    TaskBoardCard {
        id: "card-1".to_string(),
        title: title.to_string(),
        status: TaskCardStatus::InProgress,
        objective: None,
        plan: Vec::new(),
        assigned_agent: None,
        allowed_tools: Vec::new(),
        approval_mode: None,
        acceptance_criteria: Vec::new(),
        evidence: Vec::new(),
        notes: None,
        blocker: None,
        session_thread_id: None,
        source_metadata: None,
        order: 0,
        updated_at: String::new(),
    }
}

fn temp_ws() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("task-session-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn creates_top_level_tasks_thread_and_seeds_prompt() {
    let ws = temp_ws();
    let id = create_session_thread(
        ws.clone(),
        &card("Design the onboarding"),
        "run-1",
        "Do the thing",
    )
    .expect("thread created");

    // Top-level (no parent) + labelled `tasks` so it lands in the Tasks tab.
    let threads = conversations::list_threads(ws.clone()).expect("list threads");
    let t = threads.iter().find(|t| t.id == id).expect("thread listed");
    assert!(
        t.parent_thread_id.is_none(),
        "session thread must be top-level"
    );
    assert!(
        t.labels.iter().any(|l| l == "tasks"),
        "must carry the tasks label"
    );

    // Seed user message carries the prompt + correlation metadata.
    let msgs = conversations::get_messages(ws, &id).expect("messages");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].sender, "user");
    assert_eq!(msgs[0].content, "Do the thing");
}

#[test]
fn append_final_writes_agent_outcome_keyed_by_run_id() {
    let ws = temp_ws();
    let id = create_session_thread(ws.clone(), &card("X"), "run-2", "prompt").expect("thread");
    append_final(ws.clone(), &id, "run-2", &Ok("All done.".to_string()));

    let msgs = conversations::get_messages(ws, &id).expect("messages");
    let last = msgs.last().expect("has messages");
    // `agent` is the sender every renderer keys on; `assistant` used to land
    // here and painted the closing reply as a USER bubble (#5933).
    assert_eq!(last.sender, "agent");
    assert_eq!(last.content, "All done.");
    // Deterministic per run so a client that also persists the announced
    // reply under `agent:<request_id>` collapses onto this row.
    assert_eq!(last.id, "agent:run-2");
    assert_eq!(last.extra_metadata["requestId"], "run-2");
    assert_eq!(last.extra_metadata["success"], true);
    assert_eq!(last.extra_metadata["scope"], "autonomous_task_result");
}

#[test]
fn append_final_records_failure_as_unsuccessful_agent_message() {
    let ws = temp_ws();
    let id = create_session_thread(ws.clone(), &card("X"), "run-5", "prompt").expect("thread");
    append_final(ws.clone(), &id, "run-5", &Err("boom".to_string()));

    let msgs = conversations::get_messages(ws, &id).expect("messages");
    let last = msgs.last().expect("has messages");
    assert_eq!(last.sender, "agent");
    assert_eq!(last.id, "agent:run-5");
    assert_eq!(last.content, "Run failed: boom");
    assert_eq!(last.extra_metadata["success"], false);
}

#[test]
fn append_final_is_idempotent_per_run() {
    // The core persists first, then a viewing client persists the announced
    // reply under the same id — the store must keep exactly one row.
    let ws = temp_ws();
    let id = create_session_thread(ws.clone(), &card("X"), "run-4", "prompt").expect("thread");
    append_final(ws.clone(), &id, "run-4", &Ok("All done.".to_string()));
    append_final(ws.clone(), &id, "run-4", &Ok("All done.".to_string()));

    let msgs = conversations::get_messages(ws, &id).expect("messages");
    assert_eq!(
        msgs.iter().filter(|m| m.sender == "agent").count(),
        1,
        "a second append for the same run must not add a second closing message"
    );
}

#[test]
fn append_final_skips_empty_response() {
    let ws = temp_ws();
    let id = create_session_thread(ws.clone(), &card("X"), "run-3", "prompt").expect("thread");
    append_final(ws.clone(), &id, "run-3", &Ok("   ".to_string()));

    let msgs = conversations::get_messages(ws, &id).expect("messages");
    assert_eq!(
        msgs.len(),
        1,
        "empty final response must not append a message"
    );
}

#[test]
fn empty_title_falls_back_to_generic_label() {
    assert_eq!(session_title(&card("   ")), "Autonomous task");
    assert_eq!(session_title(&card("Real title")), "Real title");
}

/// The closing row that lands first is the one that survives, and the loser's
/// content is discarded — so the *order* in `run_autonomous` decides which
/// writer's text a reader sees.
///
/// `append_message` is idempotent by id and returns the **stored** row, so the
/// second write of `agent:<run_id>` is dropped whole, not merged. That is
/// correct and is what collapses the duplicate in #5933, but it means the
/// persist-before-announce ordering is load-bearing beyond tidiness: if the
/// terminal event were announced first, a viewing client would persist
/// `chat_done`'s text and the core's later `append_final` of a *failure* would
/// be silently discarded, leaving a thread that claims the run succeeded.
///
/// Pinned here rather than by driving `run_autonomous`, which needs a live
/// agent. This is the property that would break if the two statements were
/// swapped; the existing `append_final_is_idempotent_per_run` writes the same
/// content twice and so cannot see it.
#[test]
fn the_first_closing_row_wins_and_a_later_same_id_append_is_discarded() {
    let ws = temp_ws();
    let id = create_session_thread(ws.clone(), &card("X"), "run-6", "prompt").expect("thread");

    // The core persists the real outcome first — here, a failure.
    append_final(ws.clone(), &id, "run-6", &Err("boom".to_string()));
    // A viewing client then persists what `chat_done` carried, same id.
    append_final(ws.clone(), &id, "run-6", &Ok("All done.".to_string()));

    let agent_rows: Vec<_> = conversations::get_messages(ws, &id)
        .expect("messages")
        .into_iter()
        .filter(|m| m.sender == "agent")
        .collect();

    assert_eq!(agent_rows.len(), 1, "still exactly one closing row");
    assert_eq!(
        agent_rows[0].content, "Run failed: boom",
        "the row that landed first must survive; a later same-id append must \
         not overwrite a recorded failure with a success"
    );
    assert_eq!(agent_rows[0].extra_metadata["success"], false);
}
