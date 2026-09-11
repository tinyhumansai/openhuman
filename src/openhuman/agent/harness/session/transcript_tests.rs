use super::*;
use crate::openhuman::inference::provider::ToolCall;
use tempfile::TempDir;

fn sample_messages() -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(
            "You are a helpful assistant.\n\n## Tools\n\n- **shell**: Run commands",
        ),
        ChatMessage::user("What files are in /tmp?"),
        ChatMessage::assistant("Let me check that for you."),
        ChatMessage::tool("{\"tool_call_id\":\"tc1\",\"content\":\"file1.txt\\nfile2.txt\"}"),
        ChatMessage::assistant("There are two files: file1.txt and file2.txt."),
    ]
}

fn sample_meta() -> TranscriptMeta {
    TranscriptMeta {
        agent_name: "code_executor".into(),
        agent_id: Some("code_executor".into()),
        agent_type: Some("subagent".into()),
        dispatcher: "native".into(),
        provider: Some("openhuman-backend".into()),
        model: Some("claude-sonnet-4-6".into()),
        created: "2026-04-11T14:30:00Z".into(),
        updated: "2026-04-11T14:35:22Z".into(),
        turn_count: 3,
        input_tokens: 5000,
        output_tokens: 1200,
        cached_input_tokens: 3500,
        charged_amount_usd: 0.0045,
        thread_id: None,
        task_id: Some("task-123".into()),
    }
}

fn sample_turn_usage() -> TurnUsage {
    TurnUsage {
        provider: "openhuman-backend".into(),
        model: "claude-sonnet-4-6".into(),
        usage: MessageUsage {
            input: 1234,
            output: 567,
            cached_input: 1000,
            context_window: 200_000,
            cost_usd: 0.0012,
        },
        ts: "2026-04-17T10:00:00Z".into(),
        reasoning_content: Some("private reasoning trace".into()),
        tool_calls: vec![ToolCall {
            id: "call-1".into(),
            name: "shell".into(),
            arguments: "{\"cmd\":\"ls\"}".into(),
            extra_content: None,
        }],
        iteration: 1,
    }
}

// ── Phase A: append-only + compaction + display + interrupted ─────────

/// A helper mirroring the in-process persist loop: track the previously
/// persisted logical set and feed each turn through `append_transcript_turn`.
struct AppendHarness {
    path: std::path::PathBuf,
    prev: Vec<ChatMessage>,
}

impl AppendHarness {
    fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            prev: Vec::new(),
        }
    }

    fn turn(
        &mut self,
        messages: &[ChatMessage],
        meta: &TranscriptMeta,
        usage: Option<&TurnUsage>,
        request_id: Option<&str>,
    ) {
        append_transcript_turn(&self.path, &self.prev, messages, meta, usage, request_id)
            .expect("append turn");
        self.prev = messages.to_vec();
    }
}

fn roles(messages: &[ChatMessage]) -> Vec<&str> {
    messages.iter().map(|m| m.role.as_str()).collect()
}

#[path = "transcript_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "transcript_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "transcript_tests_part_03_tests.rs"]
mod part_03_tests;
