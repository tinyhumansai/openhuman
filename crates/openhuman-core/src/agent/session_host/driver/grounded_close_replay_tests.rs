//! The grounded close builds its own request after the run loop, so it has to
//! apply the thread's pinned replay options itself: a thread that expires old
//! tool results or replays only the current turn's reasoning must not send
//! either of them to the model in the close.

use super::*;
use tinyagents_harness::reasoning_replay::ReasoningReplay;
use tinyagents_harness::result_expiry::ExpiryOverrides;
use tinyagents_harness::testkit::ScriptedModel;
use tinyinference_llm::message::{AssistantMessage, ContentBlock, Message, ToolMessage};
use tinyinference_llm::tool::ToolCall;

const OLD_RESULT: &str = "three crates, listed at length";

/// Three user turns, the first of which called a tool and reasoned about it.
fn history() -> Vec<Message> {
    vec![
        Message::user("inspect the workspace"),
        Message::Assistant(AssistantMessage {
            id: Some("assistant-1".into()),
            content: vec![
                ContentBlock::thinking("old reasoning"),
                ContentBlock::Text("calling the tool".into()),
            ],
            tool_calls: vec![ToolCall {
                id: "tool-call-1".into(),
                name: "list_directory".into(),
                arguments: serde_json::json!({"path": "."}),
                invalid: None,
            }],
            usage: None,
            origin: None,
        }),
        Message::Tool(ToolMessage {
            tool_call_id: "tool-call-1".into(),
            content: vec![ContentBlock::Text(OLD_RESULT.into())],
            trusted_verbatim: false,
            artifact: None,
        }),
        Message::assistant("three crates"),
        Message::user("and now?"),
        Message::assistant("nothing else"),
        Message::user("summarize"),
        Message::Assistant(AssistantMessage {
            id: Some("assistant-2".into()),
            content: vec![
                ContentBlock::thinking("current reasoning"),
                ContentBlock::Text("looking again".into()),
            ],
            tool_calls: Vec::new(),
            usage: None,
            origin: None,
        }),
    ]
}

fn pinned() -> SessionRunOptions {
    SessionRunOptions {
        result_expiry: ExpiryOverrides {
            enabled: Some(true),
            max_age_turns: Some(1),
            exempt_tools: Some(Vec::new()),
        },
        replay_reasoning: Some(ReasoningReplay::CurrentTurn),
        ..SessionRunOptions::default()
    }
}

/// The close request's messages, as text, and every reasoning block it carries.
async fn close_request_under(replay: &SessionRunOptions) -> (Vec<String>, Vec<String>) {
    let scripted = Arc::new(ScriptedModel::new(vec![
        tinyinference_llm::model::ModelResponse::assistant("The workspace holds three crates."),
        tinyinference_llm::model::ModelResponse::assistant("ACCEPT"),
    ]));
    let source = TurnModelSource::from_model(scripted.clone());
    close_if_needed(
        &source,
        "host-selected-model",
        0.0,
        None,
        &tinytools_agent::dialect::NativeDialect,
        &history(),
        "summarize",
        &super::blank_outcome(false),
        None,
        None,
        replay,
    )
    .await
    .expect("a blank terminal reply needs a close");
    let request = scripted.requests().remove(0);
    let reasoning = request
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::Assistant(assistant) => Some(&assistant.content),
            _ => None,
        })
        .flatten()
        .filter_map(|block| match block {
            ContentBlock::Thinking { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    (
        request.messages.iter().map(Message::text).collect(),
        reasoning,
    )
}

#[tokio::test]
async fn the_close_request_applies_the_pinned_expiry_and_reasoning_replay() {
    let (texts, reasoning) = close_request_under(&pinned()).await;
    assert!(
        !texts.iter().any(|text| text == OLD_RESULT),
        "an expired result must not reach the close request, got {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("[used list_directory:")),
        "the expired result stands as a note, got {texts:?}"
    );
    assert_eq!(
        reasoning,
        ["current reasoning"],
        "a past turn's reasoning is left out and the current turn's kept"
    );
}

#[tokio::test]
async fn the_close_request_of_a_thread_with_no_pinned_options_is_unchanged() {
    let (texts, reasoning) = close_request_under(&SessionRunOptions::default()).await;
    assert!(texts.iter().any(|text| text == OLD_RESULT));
    assert_eq!(reasoning, ["old reasoning", "current reasoning"]);
}
