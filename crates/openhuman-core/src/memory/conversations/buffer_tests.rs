use super::*;
use chrono::TimeZone;

fn at(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + secs, 0).single().unwrap()
}

fn turn(thread: &str, user: &str, assistant: &str, secs: i64) -> CommittedTurn {
    CommittedTurn {
        thread_id: thread.to_string(),
        agent_id: None,
        workspace: None,
        channel: None,
        user: user.to_string(),
        assistant: assistant.to_string(),
        tool_calls: Vec::new(),
        at: at(secs),
    }
}

#[test]
fn push_flushes_exactly_when_the_batch_is_full() {
    let mut buffer = ConversationBuffer::default();
    assert!(buffer.push(turn("t", "a", "b", 0), 4, 3).is_none());
    assert!(buffer.push(turn("t", "c", "d", 1), 5, 3).is_none());
    assert_eq!(buffer.pending("t"), 2);
    let batch = buffer.push(turn("t", "e", "f", 2), 6, 3).expect("full");
    assert_eq!(batch.thread_id, "t");
    assert_eq!(batch.first, 4);
    assert_eq!(batch.last(), 6);
    assert_eq!(batch.turns.len(), 3);
    assert_eq!(buffer.pending("t"), 0);

    // The next batch starts at the index it is handed.
    assert!(buffer.push(turn("t", "g", "h", 3), 7, 3).is_none());
    let rest = buffer.take_all();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].first, 7);
}

#[test]
fn threads_are_batched_independently() {
    let mut buffer = ConversationBuffer::default();
    assert!(buffer.push(turn("a", "1", "1", 0), 0, 2).is_none());
    assert!(buffer.push(turn("b", "1", "1", 0), 0, 2).is_none());
    let full = buffer
        .push(turn("a", "2", "2", 1), 1, 2)
        .expect("a is full");
    assert_eq!(full.thread_id, "a");
    assert_eq!(buffer.pending("a"), 0);
    assert_eq!(buffer.pending("b"), 1);
}

#[test]
fn a_zero_batch_size_still_flushes_every_turn() {
    let mut buffer = ConversationBuffer::default();
    assert!(buffer.push(turn("t", "a", "b", 0), 0, 0).is_some());
}

#[test]
fn take_idle_returns_only_quiet_threads_at_the_boundary() {
    let mut buffer = ConversationBuffer::default();
    buffer.push(turn("quiet", "a", "b", 0), 0, 10);
    buffer.push(turn("busy", "a", "b", 50), 0, 10);
    // 59s after the quiet thread's last turn: not yet idle for 60s.
    assert!(buffer.take_idle(at(59), 60).is_empty());
    let ready = buffer.take_idle(at(60), 60);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].thread_id, "quiet");
    assert_eq!(buffer.pending("quiet"), 0);
    assert_eq!(buffer.pending("busy"), 1);
    let later = buffer.take_idle(at(200), 60);
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].thread_id, "busy");
}

#[test]
fn a_new_turn_resets_the_idle_clock() {
    let mut buffer = ConversationBuffer::default();
    buffer.push(turn("t", "a", "b", 0), 0, 10);
    buffer.push(turn("t", "c", "d", 100), 1, 10);
    assert!(buffer.take_idle(at(120), 60).is_empty());
    assert_eq!(buffer.take_idle(at(160), 60).len(), 1);
}

#[test]
fn take_all_and_pending_on_an_empty_buffer() {
    let mut buffer = ConversationBuffer::default();
    assert_eq!(buffer.pending("nobody"), 0);
    assert!(buffer.take_all().is_empty());
    assert!(buffer.take_idle(at(0), 1).is_empty());
}

#[test]
fn channel_tag_is_lowercased() {
    assert_eq!(channel_tag("Telegram"), "channel:telegram");
}

#[test]
fn into_item_carries_meta_and_keeps_tool_call_names_and_ids_only() {
    let mut first = turn("thread-9", "list files", "here they are", 0);
    first.agent_id = Some("orchestrator".into());
    first.workspace = Some("/work".into());
    first.channel = Some("Web".into());
    first.tool_calls = vec![
        ToolCallRef {
            name: "shell".into(),
            id: Some("call-1".into()),
        },
        ToolCallRef {
            name: "file_read".into(),
            id: None,
        },
    ];
    let second = turn("thread-9", "thanks", "you're welcome", 30);
    let item = Batch {
        thread_id: "thread-9".into(),
        first: 10,
        turns: vec![first, second],
    }
    .into_item();

    let StoreItem::Conversation { turns, meta } = item else {
        panic!("expected a conversation");
    };
    assert_eq!(meta.thread_id.as_deref(), Some("thread-9"));
    assert_eq!(meta.agent_id.as_deref(), Some("orchestrator"));
    assert_eq!(meta.workspace.as_deref(), Some("/work"));
    assert_eq!(
        meta.turns,
        Some(TurnRange {
            first: 10,
            last: 11
        })
    );
    assert_eq!(meta.source.kind, SourceKind::Conversation);
    assert_eq!(meta.source.id.as_deref(), Some("thread-9"));
    assert_eq!(meta.tags, vec!["channel:web".to_string()]);
    assert_eq!(meta.observed_at, Some(at(30)));

    assert_eq!(turns.len(), 4);
    assert_eq!(turns[0].role, Role::User);
    assert_eq!(turns[1].role, Role::Assistant);
    assert_eq!(turns[1].tool_calls.len(), 2);
    assert_eq!(turns[1].tool_calls[0].name, "shell");
    assert_eq!(turns[1].tool_calls[0].id.as_deref(), Some("call-1"));
    assert_eq!(turns[1].tool_calls[1].id, None);
    assert!(turns[0].tool_calls.is_empty());
    // The serialised item has no place for arguments.
    let json = serde_json::to_string(&turns[1].tool_calls).unwrap();
    assert!(
        !json.contains("arguments") && !json.contains("args"),
        "{json}"
    );
}

#[test]
fn into_item_skips_blank_sides_and_labels_tool_only_replies() {
    let mut tool_only = turn("t", "run it", "   ", 0);
    tool_only.tool_calls = vec![ToolCallRef {
        name: "shell".into(),
        id: Some("c".into()),
    }];
    let silent = turn("t", "   ", "just talking", 1);
    let StoreItem::Conversation { turns, meta } = (Batch {
        thread_id: "t".into(),
        first: 0,
        turns: vec![tool_only, silent],
    })
    .into_item() else {
        panic!("expected a conversation");
    };
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[1].text, "(tool calls only)");
    assert_eq!(turns[2].role, Role::Assistant);
    assert_eq!(turns[2].text, "just talking");
    assert!(meta.tags.is_empty(), "no channel, no tag");
}

#[test]
fn last_of_a_single_turn_batch_is_its_first() {
    let batch = Batch {
        thread_id: "t".into(),
        first: 5,
        turns: vec![turn("t", "a", "b", 0)],
    };
    assert_eq!(batch.last(), 5);
}
