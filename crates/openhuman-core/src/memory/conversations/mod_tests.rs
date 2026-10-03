use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use chrono::Duration;
use tinymemory::{ItemKind, MetaFilter, ToolCallRef};

fn turn(thread: &str, text: &str, at: DateTime<Utc>) -> CommittedTurn {
    CommittedTurn {
        thread_id: thread.to_string(),
        agent_id: Some("orchestrator".into()),
        workspace: Some("/work".into()),
        channel: Some("web".into()),
        user: text.to_string(),
        assistant: format!("re: {text}"),
        tool_calls: vec![ToolCallRef {
            name: "shell".into(),
            id: Some("call-1".into()),
        }],
        at,
    }
}

async fn conversations_in(
    engine: &tinymemory::conformance::ReferenceEngine,
) -> Vec<tinymemory::Hit> {
    stored(
        engine,
        MetaFilter {
            kinds: vec![ItemKind::Conversation],
            ..MetaFilter::default()
        },
    )
    .await
}

#[tokio::test]
async fn a_full_batch_is_stored_as_one_conversation_item() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 2;
    let engine = bind_reference(&config);
    let now = Utc::now();

    record_turn(&config, turn("t-batch", "first question", now)).await;
    assert_eq!(pending_turns(&config.workspace_dir, "t-batch"), 1);
    assert!(conversations_in(&engine).await.is_empty(), "not yet");

    record_turn(&config, turn("t-batch", "second question", now)).await;
    assert_eq!(pending_turns(&config.workspace_dir, "t-batch"), 0);
    let items = conversations_in(&engine).await;
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.meta.thread_id.as_deref(), Some("t-batch"));
    assert_eq!(item.meta.turns.map(|t| (t.first, t.last)), Some((0, 1)));
    assert!(item.text.contains("first question") && item.text.contains("second question"));

    let recent = recent(&config.workspace_dir);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].thread_id, "t-batch");
    assert_eq!(recent[0].turns, 2);

    // The next batch continues the thread's numbering.
    record_turn(&config, turn("t-batch", "third", now)).await;
    record_turn(&config, turn("t-batch", "fourth", now)).await;
    let items = conversations_in(&engine).await;
    assert_eq!(items.len(), 2);
    assert!(items
        .iter()
        .any(|i| i.meta.turns.map(|t| (t.first, t.last)) == Some((2, 3))));
}

#[tokio::test]
async fn idle_threads_are_flushed_without_waiting_on_a_clock() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 50;
    config.memory.conversations.idle_secs = 120;
    let engine = bind_reference(&config);
    let started = Utc::now();

    record_turn(&config, turn("t-idle", "hello", started)).await;
    assert_eq!(
        flush_idle(&config, started + Duration::seconds(119)).await,
        0
    );
    assert!(conversations_in(&engine).await.is_empty());
    assert_eq!(pending_turns(&config.workspace_dir, "t-idle"), 1);

    assert_eq!(
        flush_idle(&config, started + Duration::seconds(120)).await,
        1
    );
    assert_eq!(conversations_in(&engine).await.len(), 1);
    assert_eq!(pending_turns(&config.workspace_dir, "t-idle"), 0);
    assert_eq!(
        flush_idle(&config, started + Duration::seconds(500)).await,
        0
    );
}

#[tokio::test]
async fn flush_all_stores_every_pending_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 50;
    let engine = bind_reference(&config);
    record_turn(&config, turn("t-one", "a", Utc::now())).await;
    record_turn(&config, turn("t-two", "b", Utc::now())).await;
    assert_eq!(flush_all(&config).await, 2);
    assert_eq!(conversations_in(&engine).await.len(), 2);
    assert_eq!(flush_all(&config).await, 0);
}

#[tokio::test]
async fn disabled_conversations_and_memory_off_are_no_ops() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 1;
    let engine = bind_reference(&config);

    config.memory.conversations.enabled = false;
    record_turn(&config, turn("t-disabled", "x", Utc::now())).await;
    assert_eq!(pending_turns(&config.workspace_dir, "t-disabled"), 0);
    assert!(conversations_in(&engine).await.is_empty());
    assert!(!state_path(&config.workspace_dir).exists());

    let tmp_off = tempfile::tempdir().unwrap();
    let mut off = config_in(&tmp_off);
    off.memory.conversations.batch_turns = 1;
    record_turn(&off, turn("t-off", "x", Utc::now())).await;
    assert_eq!(pending_turns(&off.workspace_dir, "t-off"), 0);
    assert!(recent(&off.workspace_dir).is_empty());
}

#[tokio::test]
async fn stored_conversations_carry_tool_names_and_ids_but_no_arguments() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 1;
    let engine = bind_reference(&config);
    record_turn(&config, turn("t-tools", "run ls", Utc::now())).await;
    let items = conversations_in(&engine).await;
    assert_eq!(items.len(), 1);
    let json = serde_json::to_string(&items[0]).unwrap();
    assert!(json.contains("shell"), "{json}");
    assert!(json.contains("call-1"), "{json}");
    assert!(!json.contains("arguments"), "{json}");
    // Turn text never touches the bookkeeping file.
    let state = std::fs::read_to_string(state_path(&config.workspace_dir)).unwrap();
    assert!(!state.contains("run ls"), "{state}");
}

#[tokio::test]
async fn a_failed_store_drops_the_batch_without_recording_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 1;
    // A conversation with blank text on both sides is invalid for the engine.
    let mut blank = turn("t-blank", "  ", Utc::now());
    blank.assistant = String::new();
    blank.tool_calls.clear();
    bind_reference(&config);
    record_turn(&config, blank).await;
    assert_eq!(pending_turns(&config.workspace_dir, "t-blank"), 0);
    assert!(recent(&config.workspace_dir).is_empty());
}

#[tokio::test]
async fn recent_is_capped_and_newest_first() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 1;
    bind_reference(&config);
    for n in 0..RECENT_LIMIT + 3 {
        record_turn(&config, turn(&format!("t-{n}"), "msg", Utc::now())).await;
    }
    let listed = recent(&config.workspace_dir);
    assert_eq!(listed.len(), RECENT_LIMIT);
    assert_eq!(listed[0].thread_id, format!("t-{}", RECENT_LIMIT + 2));
}

#[test]
fn turn_counters_are_per_thread_and_survive_in_the_state_file() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let now = Utc::now();
    assert_eq!(next_turn_index(ws, "a", now), 0);
    assert_eq!(next_turn_index(ws, "a", now), 1);
    assert_eq!(next_turn_index(ws, "b", now), 0);
    assert_eq!(next_turn_index(ws, "a", now), 2);
}

#[test]
fn thread_counters_are_pruned_oldest_first() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let base = Utc::now() - Duration::days(1);
    let mut state = ConversationsState::default();
    for n in 0..THREAD_COUNTER_LIMIT {
        state.threads.insert(
            format!("old-{n:04}"),
            ThreadCounter {
                next: 1,
                touched_at: base + Duration::seconds(n as i64),
            },
        );
    }
    write_state(ws, &state);
    assert_eq!(next_turn_index(ws, "fresh", Utc::now()), 0);
    let after = read_state(ws);
    assert_eq!(after.threads.len(), THREAD_COUNTER_LIMIT);
    assert!(after.threads.contains_key("fresh"));
    assert!(
        !after.threads.contains_key("old-0000"),
        "the oldest went first"
    );
}

#[test]
fn an_unreadable_state_file_starts_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("memory")).unwrap();
    std::fs::write(state_path(tmp.path()), "{not json").unwrap();
    assert!(recent(tmp.path()).is_empty());
    assert_eq!(next_turn_index(tmp.path(), "t", Utc::now()), 0);
}

#[tokio::test]
async fn forget_channel_removes_only_that_channels_conversations() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.conversations.batch_turns = 1;
    let engine = bind_reference(&config);
    let mut telegram = turn("t-tg", "from telegram", Utc::now());
    telegram.channel = Some("Telegram".into());
    record_turn(&config, telegram).await;
    record_turn(&config, turn("t-web", "from web", Utc::now())).await;
    assert_eq!(conversations_in(&engine).await.len(), 2);

    assert_eq!(forget_channel(&config, "telegram").await.unwrap(), 1);
    let left = conversations_in(&engine).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].meta.thread_id.as_deref(), Some("t-web"));

    let tmp_off = tempfile::tempdir().unwrap();
    assert_eq!(
        forget_channel(&config_in(&tmp_off), "telegram")
            .await
            .unwrap(),
        0,
        "memory off forgets nothing and is not an error"
    );
}

#[test]
fn view_and_apply_set_validate_and_report() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    for bad in [
        ConversationsSetParams {
            batch_turns: Some(0),
            ..ConversationsSetParams::default()
        },
        ConversationsSetParams {
            batch_turns: Some(MAX_BATCH_TURNS + 1),
            ..ConversationsSetParams::default()
        },
        ConversationsSetParams {
            idle_secs: Some(0),
            ..ConversationsSetParams::default()
        },
        ConversationsSetParams {
            idle_secs: Some(MAX_IDLE_SECS + 1),
            ..ConversationsSetParams::default()
        },
    ] {
        assert_eq!(
            apply_set(&mut config, &bad).unwrap_err().code(),
            crate::memory::error::INVALID_REQUEST
        );
    }
    apply_set(
        &mut config,
        &ConversationsSetParams {
            enabled: Some(false),
            batch_turns: Some(7),
            idle_secs: Some(33),
        },
    )
    .unwrap();
    let view = view(&config);
    assert!(!view.enabled);
    assert_eq!(view.batch_turns, 7);
    assert_eq!(view.idle_secs, 33);
    assert!(view.recent.is_empty());
}
