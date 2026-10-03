//! #6721: a thread whose persisted head opens on orphaned `tool` rows heals on
//! its next turn. Split from `runtime_adapter_tests.rs` for the layout gate.

use std::sync::Arc;

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
    let _ = crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins();
    let root = tempfile::tempdir().expect("tempdir");
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
    let transcripts: Vec<_> = walkdir::WalkDir::new(root.path().join("workspace/session_raw"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(transcripts.len(), 1, "one head transcript: {transcripts:?}");
    let rows: Vec<serde_json::Value> = std::fs::read_to_string(&transcripts[0])
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let head_end = rows
        .iter()
        .position(|row| row.get("_meta").is_none() && row["role"] != "system")
        .expect("turn 1 wrote a non-system row");
    assert!(head_end >= 2, "fixture needs the persisted system prefix");
    let stuck = [
        serde_json::json!({"id": "call_orphan_a", "role": "tool", "content": "ra", "failure": true}),
        serde_json::json!({"id": "call_orphan_b", "role": "tool", "content": "rb"}),
        serde_json::json!({"role": "system", "content": NUDGE}),
        // The writer's native form: the calls live in a `{content, tool_calls}`
        // envelope serialised into `content`, mirrored at the top level.
        serde_json::json!({"role": "assistant", "content": serde_json::json!({
            "content": "", "tool_calls": [{"id": "call_kept", "name": "search", "arguments": "{}"}]
        }).to_string(), "tool_calls": [
            {"id": "call_kept", "name": "search", "arguments": "{}"}
        ]}),
        serde_json::json!({"id": "call_kept", "role": "tool", "content": "rk"}),
        serde_json::json!({"role": "assistant", "content": "done"}),
    ];
    let spliced: Vec<String> = rows[..head_end]
        .iter()
        .chain(stuck.iter())
        .chain(rows[head_end..].iter())
        .map(|row| row.to_string())
        .collect();
    std::fs::write(&transcripts[0], spliced.join("\n") + "\n").unwrap();

    let mut host = new_host();
    host.set_thread_id(Some("thread-orphaned-head"));
    // Self-proving fixture: the real codec must decode the orphans as `Tool`
    // rows right after the system prefix, or the request assertions below
    // would pass without the repair ever having anything to drop.
    assert!(host.resume_bound_session().await.unwrap());
    let decoded: Vec<Message> = host
        .runtime_session
        .as_ref()
        .expect("resumed runtime session")
        .history()
        .to_vec();
    let prefix = decoded
        .iter()
        .take_while(|m| matches!(m, Message::System(_)))
        .count();
    let orphan_ids: Vec<&str> = decoded[prefix..]
        .iter()
        .map_while(|m| match m {
            Message::Tool(t) => Some(t.tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        orphan_ids,
        ["call_orphan_a", "call_orphan_b"],
        "the resumed head must decode the two orphans as leading Tool rows"
    );
    assert_eq!(host.turn("second message").await.unwrap(), "second reply");

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
