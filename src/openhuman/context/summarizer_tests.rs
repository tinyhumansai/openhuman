use super::*;
use crate::openhuman::providers::{ChatResponse, ToolCall, ToolResultMessage};
use async_trait::async_trait;
use std::sync::Mutex;

fn user(text: &str) -> ConversationMessage {
    ConversationMessage::Chat(ChatMessage::user(text))
}

fn assistant(text: &str) -> ConversationMessage {
    ConversationMessage::Chat(ChatMessage::assistant(text))
}

fn call(id: &str) -> ConversationMessage {
    ConversationMessage::AssistantToolCalls {
        text: None,
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: "t".into(),
            arguments: "{}".into(),
        }],
    }
}

fn result(id: &str, body: &str) -> ConversationMessage {
    ConversationMessage::ToolResults(vec![ToolResultMessage {
        tool_call_id: id.into(),
        content: body.into(),
    }])
}

/// Minimal Provider that returns a pinned reply for every call.
/// Records how many times `chat_with_history` fired so tests can
/// assert the summarizer skipped the provider round-trip when it
/// should have.
struct StubProvider {
    reply: String,
    calls: Mutex<usize>,
}

impl StubProvider {
    fn new(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
            calls: Mutex::new(0),
        }
    }
    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl Provider for StubProvider {
    async fn chat_with_system(
        &self,
        _system: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.reply.clone())
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.reply.clone())
    }

    async fn chat(
        &self,
        _request: crate::openhuman::providers::ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        *self.calls.lock().unwrap() += 1;
        Ok(ChatResponse {
            text: Some(self.reply.clone()),
            tool_calls: vec![],
            usage: None,
        })
    }
}

#[tokio::test]
async fn noop_when_history_below_keep_recent() {
    let provider = Arc::new(StubProvider::new("IRRELEVANT"));
    let summarizer = ProviderSummarizer::new(provider.clone()).with_keep_recent(10);

    let mut history = vec![user("hi"), assistant("hello")];
    let stats = summarizer
        .summarize(&mut history, "test-model")
        .await
        .unwrap();

    assert_eq!(stats.messages_removed, 0);
    assert_eq!(history.len(), 2);
    assert_eq!(provider.call_count(), 0, "must not call provider on no-op");
}

#[tokio::test]
async fn summarizes_long_history_and_replaces_head() {
    let provider = Arc::new(StubProvider::new("SUMMARY_BODY"));
    let summarizer = ProviderSummarizer::new(provider.clone()).with_keep_recent(2);

    // 6 older messages + 2 tail = 8 total; head should collapse to 1
    // system message, tail of 2 preserved.
    let mut history = vec![
        user("q1"),
        assistant("a1"),
        user("q2"),
        assistant("a2"),
        user("q3"),
        assistant("a3"),
        user("q4-tail"),
        assistant("a4-tail"),
    ];

    let stats = summarizer
        .summarize(&mut history, "test-model")
        .await
        .unwrap();

    assert_eq!(stats.messages_removed, 6);
    assert_eq!(history.len(), 3, "1 summary + 2 tail");
    assert_eq!(provider.call_count(), 1);

    // First message must be a system summary containing the stub reply.
    match &history[0] {
        ConversationMessage::Chat(m) => {
            assert_eq!(m.role, "system");
            assert!(m.content.contains("SUMMARY_BODY"));
            assert!(m.content.contains("[auto-compacted]"));
        }
        other => panic!("expected system summary, got {other:?}"),
    }
    // Tail preserved verbatim.
    match &history[1] {
        ConversationMessage::Chat(m) => assert_eq!(m.content, "q4-tail"),
        _ => panic!(),
    }
    match &history[2] {
        ConversationMessage::Chat(m) => assert_eq!(m.content, "a4-tail"),
        _ => panic!(),
    }
}

#[tokio::test]
async fn snaps_split_past_tool_result_pair() {
    // Proposed head = 3 would land between `call("t1")` and its
    // matching `result("t1")` — the snap should push it to 4 so
    // the AssistantToolCalls ↔ ToolResults pair stays together.
    let provider = Arc::new(StubProvider::new("SUMMARY"));
    let summarizer = ProviderSummarizer::new(provider.clone()).with_keep_recent(2);

    let mut history = vec![
        user("q"),
        assistant("ack"),
        call("t1"),
        result("t1", "r1"),
        user("tail-q"),
        assistant("tail-a"),
    ];

    let _ = summarizer
        .summarize(&mut history, "test-model")
        .await
        .unwrap();

    // Expect 1 summary + 2-tail + maybe nothing between. Because
    // the head was snapped to 4, the resulting history is:
    //   [system-summary, user("tail-q"), assistant("tail-a")]
    assert_eq!(history.len(), 3);
    match &history[0] {
        ConversationMessage::Chat(m) => {
            assert_eq!(m.role, "system");
            assert!(m.content.contains("SUMMARY"));
        }
        _ => panic!(),
    }
}

#[tokio::test]
async fn empty_summary_errors_and_leaves_history_untouched() {
    let provider = Arc::new(StubProvider::new("   \n\t  "));
    let summarizer = ProviderSummarizer::new(provider).with_keep_recent(1);

    let mut history = vec![user("q1"), assistant("a1"), user("q2-tail")];
    let before = history.clone();

    let err = summarizer
        .summarize(&mut history, "test-model")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("empty"));

    // History must be untouched on error.
    assert_eq!(history.len(), before.len());
}

#[test]
fn transcript_renders_all_message_variants() {
    let msgs = vec![
        user("hello"),
        assistant("hi"),
        ConversationMessage::AssistantToolCalls {
            text: Some("let me check".into()),
            tool_calls: vec![ToolCall {
                id: "1".into(),
                name: "shell".into(),
                arguments: r#"{"cmd":"ls"}"#.into(),
            }],
        },
        result("1", "file.txt"),
    ];
    let rendered = render_transcript(&msgs);
    assert!(rendered.contains("user: hello"));
    assert!(rendered.contains("assistant: hi"));
    assert!(rendered.contains("assistant: let me check"));
    assert!(rendered.contains("assistant tool_call: shell("));
    assert!(rendered.contains("tool_result(1): file.txt"));
}
