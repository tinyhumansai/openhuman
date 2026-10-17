//! Host-side transcript-view tests (the projection itself lives in
//! `tinyagents_session::transcript::view`): they drive it through OpenHuman's
//! own writer types (`TranscriptMessage`, the session codec).
//!
//! Tool-round projection tests: text-dialect rounds persisted through the real
//! session codec and writer, and transcripts written before calls rode their
//! issuing row.

use std::sync::Arc;

use crate::agent::session_store::splice_fixture::memory_locator;
use tempfile::TempDir;
use tinyagents_session::transcript::view::{
    project_records, project_thread_in, DisplayItem, ToolCallStatus,
};
use tinyagents_session::transcript::TranscriptMessage;
use tinyagents_session::transcript::{self, TranscriptLocator};

/// A thread's display records, read back through the store port.
fn display_of(
    locator: &Arc<dyn TranscriptLocator>,
    thread_id: &str,
) -> transcript::DisplaySessionTranscript {
    locator
        .root_for_thread(thread_id)
        .expect("the thread has a root transcript")
        .read_display()
        .expect("read the display records")
        .expect("display records")
}

/// Write a raw classic JSONL transcript (meta header + `body` lines) for
/// `thread_id`, the file layout older releases left behind.
fn write_raw(workspace: &std::path::Path, stem: &str, thread_id: &str, body: &[&str]) {
    let path = workspace.join("session_raw").join(format!("{stem}.jsonl"));
    std::fs::create_dir_all(path.parent().unwrap()).expect("session_raw");
    let mut buf = format!(
        r#"{{"_meta":{{"version":1,"agent":"orchestrator","dispatcher":"xml","created":"2026-09-24T00:00:00Z","updated":"2026-09-24T00:00:10Z","turn_count":1,"input_tokens":0,"output_tokens":0,"cached_input_tokens":0,"charged_amount_usd":0.0,"thread_id":"{thread_id}"}}}}"#
    );
    buf.push('\n');
    for line in body {
        buf.push_str(line);
        buf.push('\n');
    }
    std::fs::write(&path, buf).expect("write raw transcript");
}

/// A text-dialect (`xml`/`python`/`pformat`) tool turn, persisted through the
/// real runtime codec and writer, must attach each call to the assistant row
/// that issued it and pair it with its `[Tool results]` entry — so the derived
/// transcript reports settled calls as success/error, not "running".
///
/// Regression: the codec put every tool outcome of the turn on the turn-level
/// usage record, which the writer attaches to the turn's *final* assistant row.
/// The calls then landed after their own results, the results rendered as a
/// user message, and every reloaded call projected as `running` (the UI showed
/// them as cancelled).
#[test]
fn text_dialect_tool_turn_projects_calls_on_their_issuing_row_as_settled() {
    use crate::agent::session_host::OpenHumanTranscriptCodec;
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use tinyagents_runtime::{ResumeMode, TranscriptCodec, TranscriptTurnOptions};
    use tinyinference_llm::message::Message;
    use tinytools_agent::dialect::NativeToolCall;
    use tinytools_agent::dialect::{ToolResultEntry, TranscriptEntry};

    // What the session driver persists for a text dialect: the conversation
    // rendered through the dialect's replay form.
    let conversation = vec![
        TranscriptEntry::AssistantToolCalls {
            text: None,
            tool_calls: vec![
                NativeToolCall {
                    id: "call_web_search_1".into(),
                    name: "web_search_tool".into(),
                    arguments: r#"{"query":"rust async traits"}"#.into(),
                    extra_content: None,
                },
                NativeToolCall {
                    id: "call_file_read_1".into(),
                    name: "file_read".into(),
                    arguments: r#"{"path":"README.md"}"#.into(),
                    extra_content: None,
                },
            ],
            reasoning_content: None,
            extra_metadata: None,
        },
        TranscriptEntry::ToolResults(vec![
            ToolResultEntry {
                tool_call_id: "call_web_search_1".into(),
                content: "Search results for: rust async traits".into(),
                trusted_verbatim: false,
            },
            ToolResultEntry {
                tool_call_id: "call_file_read_1".into(),
                content: "unknown tool `file_read`".into(),
                trusted_verbatim: false,
            },
        ]),
        TranscriptEntry::Chat(crate::agent::message_convert::row_to_dialect_message(
            TranscriptMessage::assistant("Here is what I found."),
        )),
    ];
    let rendered = crate::agent::message_convert::provider_messages_from_conversation(
        &tinytools_agent::dialect::XmlDialect,
        &conversation,
    );
    let mut next = vec![Message::user("search the web and read the README")];
    next.extend(crate::agent::message_convert::history_to_messages(
        &rendered,
    ));

    let context = OpenHumanRunContext::new();
    {
        let mut sidecar = context.session_sidecar.lock().unwrap();
        sidecar.model_calls = 2;
        sidecar.input_tokens = 40;
        sidecar.output_tokens = 12;
        sidecar.resolved_route = Some(tinyinference_llm::model::ResolvedModelRoute {
            provider: "e2e".into(),
            model: "e2e-mock-model".into(),
            route: "e2e".into(),
        });
        for (id, name, arguments, success, content) in [
            (
                "call_web_search_1",
                "web_search_tool",
                serde_json::json!({"query": "rust async traits"}),
                true,
                "Search results for: rust async traits",
            ),
            (
                "call_file_read_1",
                "file_read",
                serde_json::json!({"path": "README.md"}),
                false,
                "unknown tool `file_read`",
            ),
        ] {
            sidecar
                .tool_outcomes
                .push(tinyagents_harness::turn_outcome::ToolCallOutcome {
                    call_id: id.into(),
                    name: name.into(),
                    arguments,
                    success,
                    content: content.into(),
                    duration_ms: 1,
                });
        }
    }
    let options = TranscriptTurnOptions {
        request_id: Some("req-xml".into()),
        thread_id: Some("thr_xml".into()),
        stream: false,
        resume: ResumeMode::Never,
        context,
    };
    let rows = OpenHumanTranscriptCodec
        .reconcile(&[], &[], &next, &options)
        .unwrap();
    let failed_result = rows
        .iter()
        .find(|row| row.role.as_str() == "user" && row.content.starts_with("[Tool results]"))
        .expect("text dialect result row");
    assert_eq!(
        failed_result
            .extra_metadata
            .as_ref()
            .and_then(|meta| meta.get("openhuman_tool_failures")),
        Some(&serde_json::json!(["call_file_read_1"])),
        "codec must preserve failed tool IDs on the result row: {failed_result:?}"
    );
    let usage = OpenHumanTranscriptCodec.turn_usage(&options).unwrap();

    let meta = transcript::TranscriptMeta {
        session_id: None,
        parent_session_id: None,
        agent_name: "orchestrator".into(),
        agent_id: Some("orchestrator".into()),
        agent_type: Some("root".into()),
        dispatcher: "xml".into(),
        provider: None,
        model: None,
        created: "2026-09-24T00:00:00Z".into(),
        updated: "2026-09-24T00:00:00Z".into(),
        turn_count: 1,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some("thr_xml".into()),
        task_id: None,
    };
    let locator = memory_locator();
    locator
        .open_stem("xml_orchestrator", meta.clone())
        .unwrap()
        .append_turn(transcript::TranscriptTurn {
            prev: &[],
            next: &rows,
            meta: &meta,
            turn_usage: usage.as_ref(),
            request_id: Some("req-xml"),
            tools: None,
        })
        .unwrap();

    // The durable rows: the calls ride the issuing row, not the final answer.
    let persisted = locator
        .open_stem("xml_orchestrator", meta.clone())
        .unwrap()
        .messages()
        .unwrap();
    let assistants: Vec<_> = persisted
        .iter()
        .filter(|m| m.role.as_str() == "assistant")
        .collect();
    assert_eq!(assistants.len(), 2);
    let issued: Vec<String> = assistants[0]
        .turn_usage
        .as_ref()
        .map(|tu| tu.tool_calls.iter().map(|c| c.id.clone()).collect())
        .unwrap_or_default();
    assert_eq!(
        issued,
        vec![
            "call_web_search_1".to_string(),
            "call_file_read_1".to_string()
        ],
        "the issuing assistant row carries its calls"
    );
    assert!(
        assistants[1]
            .turn_usage
            .as_ref()
            .is_some_and(|tu| tu.tool_calls.is_empty() && tu.usage.input == 40),
        "the final answer carries the turn's usage but none of its calls"
    );

    // The projection: calls settled with their results, no raw results bubble.
    let display = display_of(&locator, "thr_xml");
    let items = project_records(&display.records);
    let calls: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::ToolCall {
                call_id,
                name,
                result,
                status,
                ..
            } => Some((call_id.clone(), name.clone(), result.clone(), *status)),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls.len(),
        2,
        "one item per call, no duplicates: {items:?}"
    );
    assert_eq!(calls[0].0, "call_web_search_1");
    assert_eq!(calls[0].1, "web_search_tool");
    assert_eq!(
        calls[0].2.as_deref(),
        Some("Search results for: rust async traits")
    );
    assert_eq!(calls[0].3, ToolCallStatus::Success);
    assert_eq!(calls[1].0, "call_file_read_1");
    assert_eq!(calls[1].1, "file_read");
    assert_eq!(calls[1].2.as_deref(), Some("unknown tool `file_read`"));
    assert_eq!(calls[1].3, ToolCallStatus::Error);
    assert!(
        !items.iter().any(|item| matches!(
            item,
            DisplayItem::UserMessage { content, .. } if content.starts_with("[Tool results]")
        )),
        "a tool-results turn is not a user message: {items:?}"
    );
    let first_call = items
        .iter()
        .position(|i| matches!(i, DisplayItem::ToolCall { .. }))
        .unwrap();
    let final_answer = items
        .iter()
        .position(|i| {
            matches!(i, DisplayItem::AssistantMessage { content, .. } if content == "Here is what I found.")
        })
        .unwrap();
    assert!(
        first_call < final_answer,
        "calls precede the answer they fed"
    );
}

/// Transcripts already written while the codec filed a turn's calls on its
/// final row (calls recorded *after* their own results) still project each call
/// once, settled, with its real name.
#[test]
fn calls_recorded_after_their_results_project_as_settled() {
    let dir = TempDir::new().unwrap();
    let thread = "thr_late_calls";
    write_raw(
        dir.path(),
        "late_orchestrator",
        thread,
        &[
            r#"{"role":"user","content":"search and read","request_id":"R"}"#,
            r#"{"role":"assistant","content":"","request_id":"R"}"#,
            r#"{"role":"user","content":"[Tool results]\n<tool_result id=\"call_web_search_1\">\nhits\n</tool_result>\n<tool_result id=\"call_file_read_1\">\nunknown tool\n</tool_result>\n","request_id":"R"}"#,
            r#"{"role":"assistant","content":"Here is what I found.","provider":"e2e","model":"m","usage":{"input":0,"output":0,"cached_input":0,"context_window":0,"cost_usd":0.0},"ts":"2026-09-24T00:49:35Z","iteration":2,"tool_calls":[{"id":"call_web_search_1","name":"web_search_tool","arguments":"{\"query\":\"q\"}"},{"id":"call_file_read_1","name":"file_read","arguments":"{\"path\":\"p\"}"}],"request_id":"R"}"#,
        ],
    );

    let locator = memory_locator();
    tinyagents_session::legacy::import_transcripts(dir.path(), locator.as_ref()).unwrap();
    let items = project_thread_in(locator.as_ref(), thread, None, None)
        .expect("transcript")
        .items;
    let calls: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::ToolCall {
                call_id,
                name,
                args,
                status,
                ..
            } => Some((call_id.clone(), name.clone(), args.is_some(), *status)),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        vec![
            (
                "call_web_search_1".to_string(),
                "web_search_tool".to_string(),
                true,
                ToolCallStatus::Success
            ),
            (
                "call_file_read_1".to_string(),
                "file_read".to_string(),
                true,
                ToolCallStatus::Success
            ),
        ],
        "{items:?}"
    );
}

#[test]
fn tool_failure_metadata_round_trips_write_to_display_line() {
    // Full write path: a failed tool TranscriptMessage stamped with failure metadata
    // must serialise the additive `failure` line field and read back as a failed
    // display message — proving the harness → transcript → projection seam.
    let now = "2026-07-21T09:00:00Z".to_string();
    let meta = transcript::TranscriptMeta {
        session_id: None,
        parent_session_id: None,
        agent_name: "orchestrator".into(),
        agent_id: Some("orchestrator".into()),
        agent_type: Some("root".into()),
        dispatcher: "native".into(),
        provider: Some("anthropic".into()),
        model: Some("m".into()),
        created: now.clone(),
        updated: now,
        turn_count: 1,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some("thr_rt".into()),
        task_id: None,
    };

    let mut tool_msg =
        TranscriptMessage::tool(r#"{"tool_call_id":"call-1","content":"boom"}"#).with_id("call-1");
    tool_msg.tool_failure = Some(transcript::ToolFailure {
        failed: true,
        detail: Some("boom: exit 1".into()),
    });

    let messages = vec![TranscriptMessage::user("do it"), tool_msg];
    let locator = memory_locator();
    locator
        .open_stem("700_orchestrator", meta.clone())
        .unwrap()
        .replace(&messages)
        .unwrap();

    let display = display_of(&locator, "thr_rt");
    let failed = display
        .records
        .iter()
        .find_map(|r| match r {
            transcript::DisplayRecord::Message(m) if m.message.role.as_str() == "tool" => Some(m),
            _ => None,
        })
        .expect("tool display message present");
    assert!(
        failed.failure,
        "failure flag survived the write/read round trip"
    );
    assert_eq!(failed.failure_detail.as_deref(), Some("boom: exit 1"));
}
