//! #6721: a thread whose persisted head opens on orphaned `tool` rows heals on
//! its next turn. Split from `runtime_adapter_tests.rs` for the layout gate.

use std::sync::Arc;

use crate::agent::session_store::splice_fixture::{head_of, memory_locator, replace_head};
use tinyagents_session::transcript::{TranscriptMessage, TranscriptToolCall};
use tinyinference_llm::message::Message;

/// #6721: a head persisted by the old unpaired `trim_history` opens
/// `[system, system, tool, tool, system(nudge), assistant(calls), …]`. A failed
/// turn commits nothing, so without a pre-request repair every later turn
/// resends the orphans and the provider rejects it forever.
#[test]
fn a_resumed_orphaned_tool_head_is_repaired_before_the_request() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(resumed_orphaned_tool_head_is_repaired_before_the_request());
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

async fn resumed_orphaned_tool_head_is_repaired_before_the_request() {
    const NUDGE: &str = "The last call failed validation. Fix the arguments and retry.";
    // Run the production path: the app always installs the builtin agent
    // definitions (core/jsonrpc.rs), which routes a root turn through the
    // hosted harness. Without this the result depended on whether another test
    // in the process had installed them first.
    let _ = crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins();
    let root = tempfile::tempdir().expect("tempdir");
    let locator = memory_locator();
    // The native dialect resolves from the model's profile (`Native` means
    // "native if the model supports it"); a profile-less scripted model would
    // be treated as text-only, like no production model is.
    let model = Arc::new(
        tinyagents_harness::testkit::ScriptedModel::replies(vec!["first reply", "second reply"])
            .with_profile(tinyinference_llm::model::ModelProfile {
                tool_calling: true,
                ..Default::default()
            }),
    );
    let chat_model: Arc<dyn tinyinference_llm::model::ChatModel<()>> = model.clone();
    let new_host = || {
        crate::agent::SessionHostBuilder::new()
            // Splices rows into the head: the host writes through this store.
            .with_session_history_locator(Arc::clone(&locator))
            .chat_model(chat_model.clone())
            .tools(Vec::new())
            .workspace_dir(root.path().join("workspace"))
            .action_dir(root.path().to_path_buf())
            .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
            .build()
            .expect("session build")
    };
    let mut host = new_host();
    host.set_thread_id(Some("thread-orphaned-head"));
    assert_eq!(host.turn("first message").await.unwrap(), "first reply");
    drop(host);

    // Splice the stuck head into the transcript the real writer produced.
    let head = head_of(&locator, "thread-orphaned-head");
    let rows = &head.messages;
    let head_end = rows
        .iter()
        .position(|row| row.role != "system")
        .expect("turn 1 wrote a non-system row");
    assert!(head_end >= 1, "fixture needs the persisted system prefix");
    let failed = TranscriptMessage::tool("ra").with_id("call_orphan_a");
    let call = TranscriptToolCall {
        id: "call_kept".into(),
        name: "search".into(),
        arguments: "{}".into(),
        extra_content: None,
    };
    let stuck = [
        failed,
        TranscriptMessage::tool("rb").with_id("call_orphan_b"),
        TranscriptMessage::system(NUDGE),
        TranscriptMessage::assistant_with_calls("", vec![call]),
        TranscriptMessage::tool_result("call_kept", "rk"),
        TranscriptMessage::assistant("done"),
    ];
    let spliced: Vec<TranscriptMessage> = rows[..head_end]
        .iter()
        .chain(stuck.iter())
        .chain(rows[head_end..].iter())
        .cloned()
        .collect();
    replace_head(&locator, &head, &spliced);

    let mut host = new_host();
    host.set_thread_id(Some("thread-orphaned-head"));
    // The raw durable source still contains both leading orphans; the codec
    // must heal only its model-visible replay view before strict validation.
    let raw = head_of(&locator, "thread-orphaned-head");
    let raw_prefix = raw
        .messages
        .iter()
        .take_while(|row| row.role == "system")
        .count();
    assert_eq!(
        raw.messages[raw_prefix..]
            .iter()
            .take_while(|row| row.role == "tool")
            .map(|row| row.id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["call_orphan_a", "call_orphan_b"],
    );
    assert!(host.resume_bound_session().await.unwrap());
    let decoded = host
        .runtime_session
        .as_ref()
        .expect("resumed runtime session")
        .history();
    assert!(!decoded.iter().any(
        |message| matches!(message, Message::Tool(tool) if tool.tool_call_id.starts_with("call_orphan"))
    ));
    assert_eq!(
        head_of(&locator, "thread-orphaned-head").messages,
        spliced,
        "adoption must not rewrite the durable source merely on resume",
    );
    assert_eq!(host.turn("second message").await.unwrap(), "second reply");
    let committed_history = host.runtime_session.as_ref().unwrap().history().to_vec();
    let mut resumed_again = new_host();
    resumed_again.set_thread_id(Some("thread-orphaned-head"));
    assert!(resumed_again.resume_bound_session().await.unwrap());
    assert_eq!(
        resumed_again.runtime_session.as_ref().unwrap().history(),
        committed_history.as_slice(),
        "the healed committed head must retain aligned native rows across a fresh resume",
    );

    let requests = model.requests();
    let sent = &requests
        .last()
        .expect("the resumed turn reached the model")
        .messages;
    let shape: Vec<String> = sent
        .iter()
        .map(|m| match m {
            Message::System(_) => {
                format!("system({})", m.text().chars().take(24).collect::<String>())
            }
            Message::User(_) => "user".into(),
            Message::Assistant(a) => format!("assistant(calls={})", a.tool_calls.len()),
            Message::Tool(t) => format!("tool({})", t.tool_call_id),
            _ => "other".into(),
        })
        .collect();
    // The fixture must have been read back, or the assertions below are vacuous.
    assert!(
        sent.iter()
            .any(|m| matches!(m, Message::System(_)) && m.text() == NUDGE),
        "the persisted nudge must survive the repair: {shape:?}"
    );
    assert!(
        sent.iter().any(|m| matches!(
            m,
            Message::Assistant(a) if a.tool_calls.iter().any(|c| c.id == "call_kept")
        )),
        "the persisted tool-call group must be sent: {shape:?}"
    );
    let first_non_system = sent
        .iter()
        .find(|m| !matches!(m, Message::System(_)))
        .expect("a non-system message");
    assert!(
        !matches!(first_non_system, Message::Tool(_)),
        "the request must not open on an orphaned tool result, got {first_non_system:?}"
    );
    assert!(!sent
        .iter()
        .any(|m| matches!(m, Message::Tool(t) if t.tool_call_id.starts_with("call_orphan"))));
}
