use super::*;
use tempfile::tempdir;

#[test]
fn status_strings_match_serialized_statuses() {
    assert_eq!(TaskCardStatus::Todo.as_str(), "todo");
    assert_eq!(TaskCardStatus::InProgress.as_str(), "in_progress");
    assert_eq!(TaskCardStatus::Blocked.as_str(), "blocked");
    assert_eq!(TaskCardStatus::Done.as_str(), "done");
}

#[test]
fn empty_board_uses_thread_id_and_no_cards() {
    let board = TaskBoard::empty("thread-empty");
    assert_eq!(board.thread_id, "thread-empty");
    assert!(board.cards.is_empty());
    assert!(!board.updated_at.is_empty());
}

#[tokio::test]
async fn board_store_roundtrips_and_normalises_cards() {
    let dir = tempdir().expect("tempdir");
    let store = TaskBoardStore::new(dir.path().to_path_buf());
    let board = TaskBoard {
        thread_id: "thread-1".into(),
        cards: vec![
            TaskBoardCard {
                id: String::new(),
                title: "  Draft plan  ".into(),
                status: TaskCardStatus::Todo,
                objective: Some("  Ship task briefs  ".into()),
                plan: vec!["  extend schema  ".into(), "  ".into()],
                assigned_agent: Some("  planner  ".into()),
                allowed_tools: vec![" todo ".into(), "".into()],
                approval_mode: Some(TaskApprovalMode::Required),
                acceptance_criteria: vec!["  tests pass  ".into()],
                evidence: vec!["  cargo test  ".into()],
                notes: Some("  note  ".into()),
                blocker: None,
                session_thread_id: Some("   ".into()),
                source_metadata: None,
                order: 99,
                updated_at: String::new(),
            },
            TaskBoardCard {
                id: "blocked".into(),
                title: "Need approval".into(),
                status: TaskCardStatus::Blocked,
                objective: None,
                plan: Vec::new(),
                assigned_agent: None,
                allowed_tools: Vec::new(),
                approval_mode: None,
                acceptance_criteria: Vec::new(),
                evidence: Vec::new(),
                notes: Some("waiting on user".into()),
                blocker: None,
                session_thread_id: None,
                source_metadata: None,
                order: 99,
                updated_at: String::new(),
            },
        ],
        updated_at: String::new(),
    };

    let saved = store.put(board).await.expect("put");
    assert_eq!(saved.cards[0].title, "Draft plan");
    assert_eq!(
        saved.cards[0].objective.as_deref(),
        Some("Ship task briefs")
    );
    assert_eq!(saved.cards[0].plan, vec!["extend schema"]);
    assert_eq!(saved.cards[0].assigned_agent.as_deref(), Some("planner"));
    assert_eq!(saved.cards[0].allowed_tools, vec!["todo"]);
    assert_eq!(
        saved.cards[0].approval_mode,
        Some(TaskApprovalMode::Required)
    );
    assert_eq!(saved.cards[0].acceptance_criteria, vec!["tests pass"]);
    assert_eq!(saved.cards[0].evidence, vec!["cargo test"]);
    // Whitespace-only session_thread_id normalises to None so the board
    // never offers a blank "View session" jump target.
    assert_eq!(saved.cards[0].session_thread_id, None);
    assert_eq!(saved.cards[0].order, 0);
    assert!(saved.cards[0].id.starts_with("task-"));
    assert_eq!(saved.cards[1].blocker.as_deref(), Some("waiting on user"));
    assert!(
        chrono::DateTime::parse_from_rfc3339(&saved.updated_at).is_ok(),
        "board updated_at must preserve its RFC 3339 wire format"
    );

    let loaded = store.get("thread-1").await.expect("get").expect("present");
    assert_eq!(loaded.cards.len(), 2);
    assert_eq!(loaded.cards[1].status, TaskCardStatus::Blocked);
    assert!(
        chrono::DateTime::parse_from_rfc3339(&loaded.updated_at).is_ok(),
        "persisted board reads must preserve the RFC 3339 wire format"
    );
    assert!(
        loaded
            .cards
            .iter()
            .all(|card| { chrono::DateTime::parse_from_rfc3339(&card.updated_at).is_ok() }),
        "persisted card reads must preserve the RFC 3339 wire format"
    );

    assert!(store.delete("thread-1").await.expect("delete existing"));
    assert!(store.get("thread-1").await.expect("get deleted").is_none());
    assert!(!store.delete("thread-1").await.expect("delete missing"));
}

#[tokio::test]
async fn missing_board_returns_none() {
    let dir = tempdir().expect("tempdir");
    let store = TaskBoardStore::new(dir.path().to_path_buf());
    assert!(store.get("missing").await.expect("get").is_none());
}

#[tokio::test]
async fn missing_board_fallback_uses_rfc3339_timestamp() {
    let dir = tempdir().expect("tempdir");
    let board = board_for_thread(dir.path(), "missing")
        .await
        .expect("board");
    assert!(chrono::DateTime::parse_from_rfc3339(&board.updated_at).is_ok());
}

#[tokio::test]
async fn blank_thread_id_is_rejected() {
    let dir = tempdir().expect("tempdir");
    let store = TaskBoardStore::new(dir.path().to_path_buf());
    assert!(store
        .get("   ")
        .await
        .expect_err("blank id")
        .contains("thread_id"));
}

#[test]
fn normalise_recomputes_order_after_filtering_empty_titles() {
    let mut board = TaskBoard {
        thread_id: "thread-1".into(),
        cards: vec![
            TaskBoardCard {
                id: "empty".into(),
                title: "   ".into(),
                status: TaskCardStatus::Todo,
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
                order: 99,
                updated_at: String::new(),
            },
            TaskBoardCard {
                id: "real".into(),
                title: "Real".into(),
                status: TaskCardStatus::Todo,
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
                order: 99,
                updated_at: String::new(),
            },
        ],
        updated_at: String::new(),
    };

    normalise_board(&mut board);

    assert_eq!(board.cards.len(), 1);
    assert_eq!(board.cards[0].id, "real");
    assert_eq!(board.cards[0].order, 0);
}

#[tokio::test]
async fn board_for_thread_returns_empty_board_when_file_is_missing() {
    let dir = tempdir().expect("tempdir");
    let board = board_for_thread(dir.path(), " thread-2 ")
        .await
        .expect("board");
    assert_eq!(board.thread_id, "thread-2");
    assert!(board.cards.is_empty());
}

/// An undecodable crate value must fail closed so a later read/modify/write
/// operation cannot replace the authoritative row with an empty board.
#[tokio::test]
async fn undecodable_board_value_returns_error_without_overwriting_row() {
    use tinyagents_graph::todos::store::TODOS_NAMESPACE;

    let dir = tempdir().expect("tempdir");
    let store = todos_store(dir.path());
    let key = "thread-corrupt"
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let undecodable = serde_json::json!({ "not": "a board" });
    store
        .put(TODOS_NAMESPACE, &key, undecodable.clone())
        .await
        .expect("seed garbage value");

    let board_store = TaskBoardStore::new(dir.path().to_path_buf());
    let error = board_store
        .get("thread-corrupt")
        .await
        .expect_err("undecodable row must not be treated as absent");
    assert!(error.contains("decode crate task board"));
    assert_eq!(
        store.get(TODOS_NAMESPACE, &key).await.expect("raw get"),
        Some(undecodable),
        "failed live reads must leave the authoritative row untouched"
    );
}
