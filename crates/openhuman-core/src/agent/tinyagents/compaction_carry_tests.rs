use super::*;
use tinyagents_harness::summarization::{checkpoint_message, SummaryPlacement};
use tinyinference_llm::tool::ToolCall;

fn checkpoint(body: &str) -> Message {
    checkpoint_message(SummaryPlacement::User, body)
}

fn call(id: &str) -> Message {
    let mut message = Message::assistant("");
    if let Message::Assistant(assistant) = &mut message {
        assistant
            .tool_calls
            .push(ToolCall::new(id, "read", serde_json::json!({})));
    }
    message
}

#[test]
fn reads_the_checkpoint_and_the_kept_tail() {
    let compacted = vec![
        Message::system("sys"),
        checkpoint("summary"),
        call("c3"),
        Message::tool("c3", "out"),
        Message::assistant("done"),
    ];
    let carry = CompactionCarry::from_compacted_history(&compacted).expect("carry");
    assert_eq!(carry.checkpoint, checkpoint("summary"));
    assert_eq!(carry.kept_tail, 3);
    assert!(CompactionCarry::from_compacted_history(&[Message::user("hi")]).is_none());
}

#[test]
fn folds_everything_before_the_kept_tail() {
    let carry = CompactionCarry {
        checkpoint: checkpoint("summary"),
        kept_tail: 3,
    };
    let history = vec![
        Message::system("sys"),
        Message::user("task"),
        call("c1"),
        Message::tool("c1", "one"),
        call("c2"),
        Message::tool("c2", "two"),
        Message::assistant("done"),
    ];
    let compacted = carry.apply(history);
    assert_eq!(
        compacted,
        vec![
            Message::system("sys"),
            checkpoint("summary"),
            call("c2"),
            Message::tool("c2", "two"),
            Message::assistant("done"),
        ]
    );
}

#[test]
fn never_opens_the_kept_tail_with_an_orphan_tool_result() {
    // Two kept non-system messages would start at c2's result; the cut moves
    // back to include the call.
    let carry = CompactionCarry {
        checkpoint: checkpoint("summary"),
        kept_tail: 2,
    };
    let history = vec![
        Message::user("task"),
        call("c2"),
        Message::tool("c2", "two"),
        Message::assistant("done"),
    ];
    let compacted = carry.apply(history);
    assert_eq!(
        compacted,
        vec![
            checkpoint("summary"),
            call("c2"),
            Message::tool("c2", "two"),
            Message::assistant("done"),
        ]
    );
}

#[test]
fn replaces_an_older_checkpoint_and_keeps_mid_conversation_system_notes() {
    let carry = CompactionCarry {
        checkpoint: checkpoint("new"),
        kept_tail: 1,
    };
    let history = vec![
        Message::system("sys"),
        checkpoint("old"),
        Message::user("task"),
        Message::system("note"),
        Message::assistant("done"),
    ];
    let compacted = carry.apply(history);
    assert_eq!(
        compacted,
        vec![
            Message::system("sys"),
            checkpoint("new"),
            Message::system("note"),
            Message::assistant("done"),
        ]
    );
}

#[test]
fn a_history_shorter_than_the_tail_is_unchanged() {
    let carry = CompactionCarry {
        checkpoint: checkpoint("summary"),
        kept_tail: 5,
    };
    let history = vec![Message::system("sys"), Message::user("hi")];
    assert_eq!(carry.apply(history.clone()), history);
}

#[test]
fn the_last_user_message_skips_checkpoints() {
    let history = vec![Message::user("real"), checkpoint("summary"), call("c1")];
    assert_eq!(last_user_message(&history), Some(&Message::user("real")));
    assert_eq!(last_user_message(&[checkpoint("only")]), None);
}

#[test]
fn a_checkpoint_survives_the_session_transcript_round_trip() {
    // The driver hands persisted history to the harness through the native
    // transcript form; the checkpoint must still read as one on the far side,
    // or the next turn would summarize it as raw history.
    let original = checkpoint("## Goal\nship it");
    let native = tinyagents_runtime::transcript::message_to_native_transcript(&original)
        .expect("user rows convert");
    let back = crate::agent::message_convert::chat_message_to_message(&native);
    assert!(is_checkpoint(&back), "{back:?}");
    assert_eq!(back.text(), original.text());
}
