//! Projection + pagination + sanitization tests for the transcript view.

use super::project::{project_records, project_thread};
use super::types::{DisplayItem, ToolCallStatus};
use super::{get_page, DEFAULT_LIMIT};
use crate::openhuman::agent::harness::session::transcript::{self, read_transcript_display};
use crate::openhuman::agent::messages::ChatMessage;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn meta_line(thread_id: &str) -> String {
    format!(
        r#"{{"_meta":{{"version":1,"agent":"orchestrator","dispatcher":"native","created":"2026-07-21T00:00:00Z","updated":"2026-07-21T00:00:10Z","turn_count":1,"input_tokens":30,"output_tokens":13,"cached_input_tokens":0,"charged_amount_usd":0.003,"thread_id":"{thread_id}"}}}}"#
    )
}

/// Write a raw JSONL transcript (meta header + given body lines) into
/// `session_raw/{stem}.jsonl` and return the path.
fn write_raw(workspace: &Path, stem: &str, thread_id: &str, body: &[&str]) -> PathBuf {
    let path = transcript::resolve_keyed_transcript_path(workspace, stem).expect("resolve");
    write_raw_at(&path, thread_id, body);
    path
}

fn write_raw_at(path: &Path, thread_id: &str, body: &[&str]) {
    let mut buf = meta_line(thread_id);
    buf.push('\n');
    for line in body {
        buf.push_str(line);
        buf.push('\n');
    }
    std::fs::write(&path, buf).expect("write raw transcript");
}

/// A full turn: system scaffolding, a user prompt with the injected datetime
/// prefix, an assistant tool-calling step (reasoning + tool_calls), a tool
/// result, then the final assistant answer.
fn full_turn_body() -> Vec<&'static str> {
    vec![
        r#"{"role":"system","content":"[tool-policy preamble] you may use tools ..."}"#,
        r#"{"role":"user","content":"Current Date & Time: 2026-07-21 09:00:00 UTC\n\nWhat's the weather in NYC?","request_id":"req-1"}"#,
        r#"{"role":"assistant","content":"Let me check.","provider":"anthropic","model":"claude-x","usage":{"input":10,"output":5,"cached_input":0,"cost_usd":0.001},"ts":"2026-07-21T09:00:01Z","reasoning_content":"I should call the weather tool.","tool_calls":[{"id":"call-1","name":"get_weather","arguments":"{\"city\":\"NYC\"}"}],"iteration":1,"request_id":"req-1"}"#,
        r#"{"role":"tool","content":"72F and sunny","id":"call-1","request_id":"req-1"}"#,
        r#"{"role":"assistant","content":"It's 72F and sunny in NYC.","provider":"anthropic","model":"claude-x","usage":{"input":20,"output":8,"cached_input":0,"cost_usd":0.002},"ts":"2026-07-21T09:00:02Z","iteration":2,"request_id":"req-1"}"#,
    ]
}

#[test]
fn projects_turn_with_tools_reasoning_and_sanitization() {
    let dir = TempDir::new().unwrap();
    let path = write_raw(dir.path(), "100_orchestrator", "thr_w", &full_turn_body());
    let display = read_transcript_display(&path).unwrap();
    let items = project_records(&display.records);

    // Expected order: turnBoundary, userMessage, reasoning, assistant(interim),
    // toolCall(paired), assistant(final). System line dropped.
    assert_eq!(items.len(), 6, "unexpected items: {items:#?}");

    match &items[0] {
        DisplayItem::TurnBoundary { request_id } => assert_eq!(request_id, "req-1"),
        other => panic!("expected turnBoundary, got {other:?}"),
    }
    match &items[1] {
        DisplayItem::UserMessage {
            content,
            display_content,
            request_id,
        } => {
            assert!(content.starts_with("Current Date & Time:"), "raw kept");
            assert_eq!(
                display_content.as_deref(),
                Some("What's the weather in NYC?"),
                "datetime prefix stripped into displayContent"
            );
            assert_eq!(request_id.as_deref(), Some("req-1"));
        }
        other => panic!("expected userMessage, got {other:?}"),
    }
    match &items[2] {
        DisplayItem::Reasoning { text } => assert_eq!(text, "I should call the weather tool."),
        other => panic!("expected reasoning, got {other:?}"),
    }
    match &items[3] {
        DisplayItem::AssistantMessage {
            content, interim, ..
        } => {
            assert_eq!(content, "Let me check.");
            assert!(*interim, "tool-calling assistant step is interim");
        }
        other => panic!("expected interim assistantMessage, got {other:?}"),
    }
    match &items[4] {
        DisplayItem::ToolCall {
            call_id,
            name,
            args,
            result,
            status,
            failure,
        } => {
            assert_eq!(call_id, "call-1");
            assert_eq!(name, "get_weather");
            assert_eq!(
                args.as_ref()
                    .and_then(|v| v.get("city"))
                    .and_then(|v| v.as_str()),
                Some("NYC")
            );
            assert_eq!(result.as_deref(), Some("72F and sunny"), "paired by id");
            assert_eq!(*status, ToolCallStatus::Success);
            assert!(failure.is_none(), "successful tool carries no failure");
        }
        other => panic!("expected toolCall, got {other:?}"),
    }
    match &items[5] {
        DisplayItem::AssistantMessage {
            content, interim, ..
        } => {
            assert_eq!(content, "It's 72F and sunny in NYC.");
            assert!(!*interim, "final answer is not interim");
        }
        other => panic!("expected final assistantMessage, got {other:?}"),
    }
}

#[test]
fn recovers_tool_name_from_native_envelope_without_turn_usage() {
    let dir = TempDir::new().unwrap();
    let envelope = serde_json::json!({
        "content": null,
        "tool_calls": [{
            "id": "call-web-1",
            "name": "web_fetch",
            "arguments": { "url": "https://example.com" }
        }]
    })
    .to_string();
    let assistant = serde_json::json!({
        "role": "assistant",
        "content": envelope,
        "request_id": "req-native"
    })
    .to_string();
    let result =
        r#"{"role":"tool","content":"Example Domain","id":"call-web-1","request_id":"req-native"}"#;
    let path = write_raw(
        dir.path(),
        "101_orchestrator",
        "thr_native",
        &[assistant.as_str(), result],
    );
    let display = read_transcript_display(&path).unwrap();
    let items = project_records(&display.records);

    let tool = items
        .iter()
        .find_map(|item| match item {
            DisplayItem::ToolCall {
                name,
                args,
                result,
                status,
                ..
            } => Some((name, args, result, status)),
            _ => None,
        })
        .expect("native envelope tool call projected");
    assert_eq!(tool.0, "web_fetch");
    assert_eq!(
        tool.1
            .as_ref()
            .and_then(|args| args.get("url"))
            .and_then(serde_json::Value::as_str),
        Some("https://example.com")
    );
    assert_eq!(tool.2.as_deref(), Some("Example Domain"));
    assert_eq!(*tool.3, ToolCallStatus::Success);
}

#[test]
fn projects_compaction_and_interrupted_partial() {
    let dir = TempDir::new().unwrap();
    let body = vec![
        r#"{"role":"user","content":"hi","request_id":"req-1"}"#,
        r#"{"kind":"compaction","replacement":[{"role":"user","content":"summary so far"}],"ts":"2026-07-21T09:05:00Z","request_id":"req-2"}"#,
        r#"{"role":"assistant","content":"partial ans","interrupted":true,"reasoning_content":"mid-thought","iteration":3,"request_id":"req-2"}"#,
    ];
    let path = write_raw(dir.path(), "200_orchestrator", "thr_c", &body);
    let display = read_transcript_display(&path).unwrap();
    let items = project_records(&display.records);

    let has_compaction = items.iter().any(|i| {
        matches!(
            i,
            DisplayItem::Compaction { kept_count, .. } if *kept_count == 1
        )
    });
    assert!(has_compaction, "compaction projected: {items:#?}");

    let partial = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::InterruptedPartial { text, thinking } => Some((text, thinking)),
            _ => None,
        })
        .expect("interrupted partial projected");
    assert_eq!(partial.0, "partial ans");
    assert_eq!(partial.1.as_deref(), Some("mid-thought"));
}

#[test]
fn legacy_file_without_version_or_request_id_projects() {
    let dir = TempDir::new().unwrap();
    // Legacy meta: no `version`, messages carry no `request_id`.
    let path = transcript::resolve_keyed_transcript_path(dir.path(), "300_orchestrator").unwrap();
    let raw = concat!(
        r#"{"_meta":{"agent":"orchestrator","dispatcher":"native","created":"2026-01-01T00:00:00Z","updated":"2026-01-01T00:00:00Z","turn_count":1,"input_tokens":0,"output_tokens":0,"cached_input_tokens":0,"charged_amount_usd":0.0,"thread_id":"thr_legacy"}}"#,
        "\n",
        r#"{"role":"user","content":"plain question"}"#,
        "\n",
        r#"{"role":"assistant","content":"plain answer"}"#,
        "\n",
    );
    std::fs::write(&path, raw).unwrap();
    let display = read_transcript_display(&path).unwrap();
    let items = project_records(&display.records);

    // No request_id → no turn boundary; user + assistant still project, and an
    // un-prefixed user message keeps no displayContent (nothing to strip).
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, DisplayItem::TurnBoundary { .. })),
        "legacy lines have no request_id, so no boundary"
    );
    match items.first() {
        Some(DisplayItem::UserMessage {
            content,
            display_content,
            ..
        }) => {
            assert_eq!(content, "plain question");
            assert_eq!(
                display_content.as_deref(),
                None,
                "no prefix, no displayContent"
            );
        }
        other => panic!("expected userMessage first, got {other:?}"),
    }
    assert!(items.iter().any(
        |i| matches!(i, DisplayItem::AssistantMessage { content, .. } if content == "plain answer")
    ));
}

#[test]
fn subagent_file_projects_as_nested_item() {
    let dir = TempDir::new().unwrap();
    let root_stem = "400_orchestrator";
    write_raw(
        dir.path(),
        root_stem,
        "thr_s",
        &[r#"{"role":"user","content":"delegate please","request_id":"req-1"}"#],
    );
    // Sub-agent sibling shares the root stem with a `__` suffix.
    write_raw(
        dir.path(),
        &format!("{root_stem}__100_coder"),
        "thr_s",
        &[
            r#"{"role":"assistant","content":"sub work done","provider":"anthropic","model":"claude-x","usage":{"input":5,"output":3,"cached_input":0,"cost_usd":0.0},"ts":"2026-07-21T09:10:00Z","iteration":1}"#,
        ],
    );

    let projected = project_thread(dir.path(), "thr_s").expect("project thread");
    let subagent = projected
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Subagent { id, items, .. } => Some((id, items)),
            _ => None,
        })
        .expect("subagent item present");
    assert_eq!(subagent.0, "orchestrator");
    assert!(subagent.1.iter().any(
        |i| matches!(i, DisplayItem::AssistantMessage { content, .. } if content == "sub work done")
    ));
}

#[test]
fn profile_scoped_root_and_subagent_project_together() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw-alice");
    std::fs::create_dir_all(&raw_dir).unwrap();
    let root_stem = "450_orchestrator";
    let thread_id = "thr_profile";

    write_raw_at(
        &raw_dir.join(format!("{root_stem}.jsonl")),
        thread_id,
        &[r#"{"role":"user","content":"delegate","request_id":"req-1"}"#],
    );
    write_raw_at(
        &raw_dir.join(format!("{root_stem}__451_coder.jsonl")),
        thread_id,
        &[r#"{"role":"assistant","content":"scoped sub work"}"#],
    );

    let projected = project_thread(dir.path(), thread_id).expect("project scoped thread");
    assert!(projected.items.iter().any(|item| matches!(
        item,
        DisplayItem::Subagent { items, .. }
            if items.iter().any(|inner| matches!(
                inner,
                DisplayItem::AssistantMessage { content, .. }
                    if content == "scoped sub work"
            ))
    )));
}

#[test]
fn get_page_paginates_newest_first_with_cursor() {
    let dir = TempDir::new().unwrap();
    // Five plain user messages → five top-level items.
    let body: Vec<String> = (0..5)
        .map(|i| format!(r#"{{"role":"user","content":"msg-{i}"}}"#))
        .collect();
    let body_refs: Vec<&str> = body.iter().map(String::as_str).collect();
    write_raw(dir.path(), "500_orchestrator", "thr_p", &body_refs);

    // First page: newest first, limit 2 → msg-4, msg-3.
    let page1 = get_page(dir.path(), "thr_p", None, Some(2));
    assert_eq!(page1.total, 5);
    assert!(page1.has_more);
    assert_eq!(page1.items.len(), 2);
    assert!(
        matches!(&page1.items[0], DisplayItem::UserMessage { content, .. } if content == "msg-4")
    );
    assert!(
        matches!(&page1.items[1], DisplayItem::UserMessage { content, .. } if content == "msg-3")
    );

    let cursor = page1.next_cursor.clone().expect("next cursor");
    let page2 = get_page(dir.path(), "thr_p", Some(&cursor), Some(2));
    assert!(
        matches!(&page2.items[0], DisplayItem::UserMessage { content, .. } if content == "msg-2")
    );

    // Walk to the end.
    let last = get_page(dir.path(), "thr_p", page2.next_cursor.as_deref(), Some(2));
    assert!(!last.has_more, "final page exhausts the thread");
    assert!(last.next_cursor.is_none());
}

#[test]
fn failed_tool_line_projects_error_status_with_failure_payload() {
    let dir = TempDir::new().unwrap();
    // Assistant issues a tool call; the paired tool result line carries the
    // additive `failure` flag (stamped at persistence from `is_error`).
    let body = vec![
        r#"{"role":"assistant","content":"trying","provider":"anthropic","model":"m","usage":{"input":1,"output":1,"cached_input":0,"cost_usd":0.0},"ts":"2026-07-21T09:00:01Z","tool_calls":[{"id":"call-9","name":"shell","arguments":"{\"cmd\":\"boom\"}"}],"iteration":1,"request_id":"req-1"}"#,
        r#"{"role":"tool","content":"error: command not found","id":"call-9","request_id":"req-1","failure":true,"failure_detail":"error: command not found"}"#,
    ];
    let path = write_raw(dir.path(), "600_orchestrator", "thr_f", &body);
    let display = read_transcript_display(&path).unwrap();
    let items = project_records(&display.records);

    let tool = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::ToolCall {
                status, failure, ..
            } => Some((status, failure)),
            _ => None,
        })
        .expect("toolCall projected");
    assert_eq!(*tool.0, ToolCallStatus::Error, "failed tool → error status");
    let failure = tool.1.as_ref().expect("failure payload present");
    assert_eq!(failure.detail.as_deref(), Some("error: command not found"));
}

#[test]
fn tool_failure_metadata_round_trips_write_to_display_line() {
    // Full write path: a failed tool ChatMessage stamped with failure metadata
    // must serialise the additive `failure` line field and read back as a failed
    // display message — proving the harness → transcript → projection seam.
    let dir = TempDir::new().unwrap();
    let now = "2026-07-21T09:00:00Z".to_string();
    let meta = transcript::TranscriptMeta {
        agent_name: "orchestrator".into(),
        agent_id: Some("orchestrator".into()),
        agent_type: Some("root".into()),
        dispatcher: "native".into(),
        provider: Some("anthropic".into()),
        model: Some("m".into()),
        created: now.clone(),
        updated: now,
        turn_count: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some("thr_rt".into()),
        task_id: None,
    };

    let mut tool_msg = ChatMessage {
        id: Some("call-1".into()),
        role: "tool".into(),
        content: r#"{"tool_call_id":"call-1","content":"boom"}"#.into(),
        extra_metadata: None,
    };
    transcript::attach_tool_failure_metadata(&mut tool_msg, Some("boom: exit 1"));

    let messages = vec![
        ChatMessage {
            id: None,
            role: "user".into(),
            content: "do it".into(),
            extra_metadata: None,
        },
        tool_msg,
    ];
    let path = transcript::resolve_keyed_transcript_path(dir.path(), "700_orchestrator").unwrap();
    transcript::write_transcript(&path, &messages, &meta, None).unwrap();

    let display = read_transcript_display(&path).unwrap();
    let failed = display
        .records
        .iter()
        .find_map(|r| match r {
            transcript::DisplayRecord::Message(m) if m.message.role == "tool" => Some(m),
            _ => None,
        })
        .expect("tool display message present");
    assert!(
        failed.failure,
        "failure flag survived the write/read round trip"
    );
    assert_eq!(failed.failure_detail.as_deref(), Some("boom: exit 1"));
}

/// The **golden test for the turn path's write call site**.
///
/// Every other test in this file hand-writes JSONL string literals, so they pin
/// the reader/projector but say nothing about the writer the live turn loop
/// actually calls. `tool_failure_metadata_round_trips_write_to_display_line`
/// goes through `write_transcript`, not `append_transcript_turn`. That left the
/// seam this test covers — `append_transcript_turn` → `read_transcript_display`
/// → `project_thread` — with no coverage at all, which is exactly the seam the
/// tinyagents `ChatHistory` migration touches.
///
/// It fails loudly if a write ever drops `request_id` or `turn_usage`: without
/// `request_id` there is no `DisplayItem::TurnBoundary` (project.rs
/// `maybe_emit_turn_boundary`) and `turn_segments` goes empty, unanchoring every
/// sub-agent; without `turn_usage` every `DisplayItem::ToolCall` disappears
/// (tool calls are read off `turn_usage.tool_calls`), `Reasoning` vanishes, and
/// `AssistantMessage.{model,iteration,interim}` collapse to `None`/`false`.
///
/// All timestamps are fixed literals so nothing here is clock-dependent.
#[test]
fn append_transcript_turn_projects_full_display_shape() {
    let dir = TempDir::new().unwrap();
    let now = "2026-07-21T09:00:00Z".to_string();
    let meta = transcript::TranscriptMeta {
        agent_name: "orchestrator".into(),
        agent_id: Some("orchestrator".into()),
        agent_type: Some("root".into()),
        dispatcher: "native".into(),
        provider: Some("anthropic".into()),
        model: Some("claude-x".into()),
        created: now.clone(),
        updated: now,
        turn_count: 1,
        input_tokens: 30,
        output_tokens: 13,
        cached_input_tokens: 0,
        charged_amount_usd: 0.003,
        thread_id: Some("thr_golden".into()),
        task_id: None,
    };

    let usage = |cost: f64| transcript::MessageUsage {
        input: 20,
        output: 8,
        cached_input: 0,
        context_window: 200_000,
        cost_usd: cost,
    };

    // Iteration 1 of the turn: the model reasons and emits a native tool call.
    // `turn_usage` attaches to the last assistant row of the written slice, so
    // this one lands on the interim assistant.
    let interim_usage = transcript::TurnUsage {
        provider: "anthropic".into(),
        model: "claude-x".into(),
        usage: usage(0.001),
        ts: "2026-07-21T09:00:01Z".into(),
        reasoning_content: Some("I should call the weather tool.".into()),
        tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
            id: "call-1".into(),
            name: "get_weather".into(),
            arguments: r#"{"city":"NYC"}"#.into(),
            extra_content: None,
        }],
        iteration: 1,
    };
    // Iteration 2: the final answer, no further tool calls.
    let final_usage = transcript::TurnUsage {
        provider: "anthropic".into(),
        model: "claude-x".into(),
        usage: usage(0.002),
        ts: "2026-07-21T09:00:02Z".into(),
        reasoning_content: None,
        tool_calls: vec![],
        iteration: 2,
    };

    let msg = |id: Option<&str>, role: &str, content: &str| ChatMessage {
        id: id.map(str::to_string),
        role: role.into(),
        content: content.into(),
        extra_metadata: None,
    };

    let first = vec![
        msg(None, "user", "What's the weather in NYC?"),
        msg(None, "assistant", "Let me check."),
    ];
    let mut second = first.clone();
    second.push(msg(Some("call-1"), "tool", "72F and sunny"));
    second.push(msg(None, "assistant", "It's 72F and sunny in NYC."));

    let path = transcript::resolve_keyed_transcript_path(dir.path(), "900_orchestrator").unwrap();
    // First write creates the file (meta + all lines); the second is a pure
    // extension appending only the new tail — both are the real turn-path shape.
    transcript::append_transcript_turn(
        &path,
        &[],
        &first,
        &meta,
        Some(&interim_usage),
        Some("req-1"),
    )
    .unwrap();
    transcript::append_transcript_turn(
        &path,
        &first,
        &second,
        &meta,
        Some(&final_usage),
        Some("req-1"),
    )
    .unwrap();

    let display = read_transcript_display(&path).unwrap();
    let items = project_records(&display.records);

    // A turn boundary must be emitted from the stamped request_id.
    let boundary = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::TurnBoundary { request_id } => Some(request_id.clone()),
            _ => None,
        })
        .expect("turnBoundary projected from request_id");
    assert_eq!(boundary, "req-1");

    // Reasoning comes off turn_usage.reasoning_content.
    let reasoning = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Reasoning { text } => Some(text.clone()),
            _ => None,
        })
        .expect("reasoning projected from turn_usage");
    assert_eq!(reasoning, "I should call the weather tool.");

    // The tool call itself is read off turn_usage.tool_calls, and pairs with the
    // role:"tool" line by id.
    let tool = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::ToolCall {
                call_id,
                name,
                args,
                result,
                status,
                ..
            } => Some((
                call_id.clone(),
                name.clone(),
                args.clone(),
                result.clone(),
                *status,
            )),
            _ => None,
        })
        .expect("toolCall projected from turn_usage.tool_calls");
    assert_eq!(tool.0, "call-1");
    assert_eq!(tool.1, "get_weather");
    assert_eq!(
        tool.2
            .as_ref()
            .and_then(|v| v.get("city"))
            .and_then(|v| v.as_str()),
        Some("NYC")
    );
    assert_eq!(tool.3.as_deref(), Some("72F and sunny"));
    assert_eq!(tool.4, ToolCallStatus::Success);

    // Model / iteration / request_id / interim all come off the persisted
    // `turn_usage` + `request_id`; each is `None`/`false` if either is dropped.
    let assistants: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::AssistantMessage {
                content,
                model,
                iteration,
                request_id,
                interim,
                ..
            } => Some((
                content.clone(),
                model.clone(),
                *iteration,
                request_id.clone(),
                *interim,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(assistants.len(), 2, "unexpected items: {items:#?}");

    assert_eq!(assistants[0].0, "Let me check.");
    assert_eq!(assistants[0].1.as_deref(), Some("claude-x"));
    assert_eq!(assistants[0].2, Some(1));
    assert_eq!(assistants[0].3.as_deref(), Some("req-1"));
    assert!(assistants[0].4, "tool-calling step is interim");

    assert_eq!(assistants[1].0, "It's 72F and sunny in NYC.");
    assert_eq!(assistants[1].1.as_deref(), Some("claude-x"));
    assert_eq!(assistants[1].2, Some(2));
    assert_eq!(assistants[1].3.as_deref(), Some("req-1"));
    assert!(!assistants[1].4, "final answer is not interim");
}

#[test]
fn subagent_anchors_to_parent_turn_by_spawn_timestamp() {
    let dir = TempDir::new().unwrap();
    let root_stem = "800_orchestrator";
    let thread_id = "thr_anchor";

    let t1 = chrono::DateTime::from_timestamp(1_000_000, 0)
        .unwrap()
        .to_rfc3339();
    let t2 = chrono::DateTime::from_timestamp(2_000_000, 0)
        .unwrap()
        .to_rfc3339();

    // Two turns: req-1 (assistant ts t1), req-2 (assistant ts t2).
    let root_body = vec![
        r#"{"role":"user","content":"one","request_id":"req-1"}"#.to_string(),
        format!(
            r#"{{"role":"assistant","content":"a1","provider":"anthropic","model":"m","usage":{{"input":1,"output":1,"cached_input":0,"cost_usd":0.0}},"ts":"{t1}","iteration":1,"request_id":"req-1"}}"#
        ),
        r#"{"role":"user","content":"two","request_id":"req-2"}"#.to_string(),
        format!(
            r#"{{"role":"assistant","content":"a2","provider":"anthropic","model":"m","usage":{{"input":1,"output":1,"cached_input":0,"cost_usd":0.0}},"ts":"{t2}","iteration":1,"request_id":"req-2"}}"#
        ),
    ];
    let root_refs: Vec<&str> = root_body.iter().map(String::as_str).collect();
    write_raw(dir.path(), root_stem, thread_id, &root_refs);

    // Sub-agent stems encode the spawn unix timestamp: coder spawned during
    // turn 1 (1_000_050), planner during turn 2 (2_000_050).
    write_raw(
        dir.path(),
        &format!("{root_stem}__1000050_coder"),
        thread_id,
        &[r#"{"role":"assistant","content":"coder work"}"#],
    );
    write_raw(
        dir.path(),
        &format!("{root_stem}__2000050_planner"),
        thread_id,
        &[r#"{"role":"assistant","content":"planner work"}"#],
    );

    let projected = project_thread(dir.path(), thread_id).expect("project thread");
    // The seeded sub-agent files share the `orchestrator` meta agent name, so
    // key the anchoring by each sub-agent's inner work content instead of `id`.
    let mut anchors: Vec<(String, Option<String>)> = projected
        .items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Subagent {
                request_id, items, ..
            } => {
                let marker = items.iter().find_map(|inner| match inner {
                    DisplayItem::AssistantMessage { content, .. } => Some(content.clone()),
                    _ => None,
                })?;
                Some((marker, request_id.clone()))
            }
            _ => None,
        })
        .collect();
    anchors.sort();

    assert_eq!(
        anchors,
        vec![
            ("coder work".to_string(), Some("req-1".to_string())),
            ("planner work".to_string(), Some("req-2".to_string())),
        ],
        "each sub-agent anchors to the turn active at its spawn time"
    );
}

#[test]
fn get_page_missing_thread_is_empty_not_error() {
    let dir = TempDir::new().unwrap();
    let page = get_page(dir.path(), "no_such_thread", None, Some(DEFAULT_LIMIT));
    assert!(!page.has_transcript);
    assert_eq!(page.total, 0);
    assert!(page.items.is_empty());
}
