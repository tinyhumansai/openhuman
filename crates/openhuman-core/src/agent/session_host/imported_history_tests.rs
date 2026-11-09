use super::*;

use std::sync::Arc;
use tinyagents_session::transcript::TranscriptMessage;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::ChatModel;

#[test]
fn imported_history_gets_the_real_prefix_and_freezes_it_after_first_turn() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(run_imported_history_prefix_regression());
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

async fn run_imported_history_prefix_regression() {
    let root = tempfile::tempdir().expect("workspace");
    let locator = crate::storage::sessions::splice_fixture_tests::memory_locator();
    let tool = || {
        Box::new(tinyagents_harness::testkit::FakeTool::returning(
            "relay_history_probe",
            "ok",
        )) as Box<dyn tinytools::Tool>
    };
    let first_model = Arc::new(
        tinyagents_harness::testkit::ScriptedModel::replies(vec!["first reply"]).with_profile(
            tinyinference_llm::model::ModelProfile {
                tool_calling: true,
                ..Default::default()
            },
        ),
    );
    let first_chat_model: Arc<dyn ChatModel<()>> = first_model.clone();
    let mut first = SessionHostBuilder::new()
        .with_session_history_locator(Arc::clone(&locator))
        .chat_model(first_chat_model)
        .model_name("test-model".to_owned())
        .tools(vec![tool()])
        .prompt_builder(crate::agent::prompts::SystemPromptBuilder::verbatim(
            "You are the relay assistant. Preserve this exact system prefix.".to_owned(),
        ))
        .workspace_dir(root.path().join("workspace"))
        .action_dir(root.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("first session");
    first.set_thread_id(Some("relay-import-thread"));
    let imported = [
        TranscriptMessage::user("legacy relay question"),
        TranscriptMessage::assistant("legacy relay answer"),
    ];
    assert!(first.adopt_imported_history(&imported).unwrap());
    assert!(!first.adopt_imported_history(&imported).unwrap());
    assert_eq!(
        first.turn("new relay question").await.unwrap(),
        "first reply"
    );
    let actual_agent_id = first.agent_definition_id.clone();

    let first_request = first_model
        .requests()
        .into_iter()
        .next()
        .expect("first model request");
    let first_messages = &first_request.messages;
    let prefix: Vec<_> = first_messages
        .iter()
        .filter(|message| matches!(message, Message::System(_)))
        .map(Message::text)
        .collect();
    assert!(!prefix.is_empty(), "the configured system prefix is sent");
    assert!(first_request
        .tools
        .iter()
        .any(|tool| tool.name == "relay_history_probe"));
    let legacy_question = first_messages
        .iter()
        .position(|message| matches!(message, Message::User(_) if message.text() == "legacy relay question"))
        .expect("imported user message reaches first model request");
    let legacy_answer = first_messages
        .iter()
        .position(|message| matches!(message, Message::Assistant(_) if message.text() == "legacy relay answer"))
        .expect("imported assistant message reaches first model request");
    let current_question = first_messages
        .iter()
        .position(|message| matches!(message, Message::User(_) if message.text().contains("new relay question")))
        .expect("current user message reaches first model request");
    assert!(
        first_messages
            .iter()
            .position(|message| matches!(message, Message::System(_)))
            .expect("system prefix")
            < legacy_question
            && legacy_question < legacy_answer
            && legacy_answer < current_question,
        "the live prefix precedes imported rows, followed by this turn"
    );
    let session_ref =
        tinyagents_session::transcript::SessionRef::scoped("relay-import-thread", &actual_agent_id);
    let original = locator
        .read_session_transcript(&session_ref.first_generation())
        .expect("original imported generation")
        .read_session()
        .unwrap()
        .unwrap();
    assert_eq!(original.messages, imported);

    let head = locator.head_generation(&session_ref);
    assert_ne!(
        head, session_ref,
        "prefix adoption opens a successor generation"
    );
    let transcript = locator
        .read_session_transcript(&head)
        .expect("adopted session transcript")
        .read_session()
        .unwrap()
        .unwrap();
    assert!(transcript.meta.prefix_message_count.unwrap_or_default() > 0);
    assert!(transcript
        .tools
        .as_ref()
        .is_some_and(|tools| tools.to_string().contains("relay_history_probe")));
    drop(first);

    let second_model = Arc::new(
        tinyagents_harness::testkit::ScriptedModel::replies(vec!["second reply"]).with_profile(
            tinyinference_llm::model::ModelProfile {
                tool_calling: true,
                ..Default::default()
            },
        ),
    );
    let second_chat_model: Arc<dyn ChatModel<()>> = second_model.clone();
    let mut second = SessionHostBuilder::new()
        .with_session_history_locator(locator)
        .chat_model(second_chat_model)
        .model_name("test-model".to_owned())
        .tools(vec![tool()])
        .prompt_builder(crate::agent::prompts::SystemPromptBuilder::verbatim(
            "This replacement prompt must not change an existing session.".to_owned(),
        ))
        .workspace_dir(root.path().join("workspace"))
        .action_dir(root.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("resumed session");
    second.set_thread_id(Some("relay-import-thread"));
    assert_eq!(second.turn("after restart").await.unwrap(), "second reply");
    let second_request = second_model
        .requests()
        .into_iter()
        .next()
        .expect("resumed model request");
    let second_prefix: Vec<_> = second_request
        .messages
        .iter()
        .filter(|message| matches!(message, Message::System(_)))
        .map(Message::text)
        .collect();
    assert_eq!(second_prefix, prefix, "resume freezes the committed prompt");
    assert!(second_prefix
        .iter()
        .any(|message| message.contains("Preserve this exact system prefix")));
    assert!(!second_prefix
        .iter()
        .any(|message| message.contains("replacement prompt")));
    assert_eq!(
        second_request.tools, first_request.tools,
        "resume restores the committed tool snapshot"
    );
    assert!(second_request
        .tools
        .iter()
        .any(|tool| tool.name == "relay_history_probe"));
    let resumed_history = serde_json::to_string(&second_request.messages).unwrap();
    assert!(resumed_history.contains("legacy relay question"));
    assert!(resumed_history.contains("legacy relay answer"));
    assert!(resumed_history.contains("new relay question"));
    assert!(resumed_history.contains("first reply"));
}

#[tokio::test]
async fn legacy_history_is_loaded_only_for_an_unwritten_bound_session() {
    use tinyagents_session::transcript::{TranscriptMessage, TranscriptMeta};

    let root = tempfile::tempdir().expect("workspace");
    let locator = crate::storage::sessions::splice_fixture_tests::memory_locator();
    let tool = || {
        Box::new(tinyagents_harness::testkit::FakeTool::returning(
            "legacy_history_probe",
            "ok",
        )) as Box<dyn tinytools::Tool>
    };
    let model = Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
        "reply after legacy adoption",
    ]));
    let chat_model: Arc<dyn ChatModel<()>> = model.clone();
    let mut host = SessionHostBuilder::new()
        .with_session_history_locator(Arc::clone(&locator))
        .chat_model(chat_model)
        .model_name("test-model".to_owned())
        .tools(vec![tool()])
        .workspace_dir(root.path().join("workspace"))
        .action_dir(root.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("session host");

    host.set_thread_id(Some("legacy-history-lazy"));
    let session = host.bound_session_ref().expect("bound session").clone();
    assert!(!host.bound_history_is_authoritative().unwrap());

    let absent = host
        .adopt_legacy_history_if_needed(|| async {
            Err(anyhow::anyhow!("legacy store unavailable"))
        })
        .await;
    assert!(
        absent.is_err(),
        "legacy read failure must stop first admission"
    );
    assert!(
        !locator.session_exists(&session),
        "failed legacy read must not create an absent session"
    );

    // Merely opening the exact transcript handle does not count as a durable
    // turn and must not suppress the one-time legacy import.
    let now = chrono::Utc::now().to_rfc3339();
    locator
        .open_stem(
            &tinyagents_session::transcript::session_stem(&session),
            TranscriptMeta {
                agent_name: "orchestrator".into(),
                agent_id: session.agent_id.clone(),
                agent_type: Some("root".into()),
                dispatcher: "native".into(),
                provider: None,
                model: None,
                created: now.clone(),
                updated: now,
                turn_count: 0,
                prefix_message_count: None,
                prefix_reset: false,
                input_tokens: 0,
                output_tokens: 0,
                cached_input_tokens: 0,
                charged_amount_usd: 0.0,
                thread_id: Some(session.session_key.clone()),
                task_id: None,
                session_id: Some(session.session_id()),
                parent_session_id: None,
            },
        )
        .unwrap();
    assert!(!host.bound_history_is_authoritative().unwrap());
    let missing = host
        .adopt_legacy_history_if_needed(|| async {
            Err(anyhow::anyhow!("legacy store unavailable"))
        })
        .await;
    assert!(
        missing.is_err(),
        "failed legacy reads stop before a turn starts"
    );
    assert!(
        !locator.session_exists(&session),
        "a failed import must not create an empty durable transcript"
    );

    let imported = vec![TranscriptMessage::user("legacy question")];
    assert!(host
        .adopt_legacy_history_if_needed(|| async { Ok(imported.clone()) })
        .await
        .unwrap());
    assert!(host.bound_history_is_authoritative().unwrap());
    assert!(!host
        .adopt_legacy_history_if_needed(|| async {
            Err(anyhow::anyhow!("legacy store became unavailable"))
        })
        .await
        .unwrap());
    assert_eq!(
        host.turn("first durable turn").await.unwrap(),
        "reply after legacy adoption"
    );
    let first_request = model.requests().into_iter().next().expect("first request");
    let first_messages = serde_json::to_string(&first_request.messages).unwrap();
    assert!(first_messages.contains("legacy question"));
    assert!(first_messages.contains("first durable turn"));

    // A separate host resumes the committed turn while the legacy source is
    // unavailable. It must use the durable transcript and never read legacy.
    let second_root = tempfile::tempdir().expect("second workspace");
    let second_model = Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
        "reply after restart",
    ]));
    let second_chat_model: Arc<dyn ChatModel<()>> = second_model.clone();
    let mut resumed = SessionHostBuilder::new()
        .with_session_history_locator(locator.clone())
        .chat_model(second_chat_model)
        .model_name("test-model".to_owned())
        .tools(vec![tool()])
        .workspace_dir(second_root.path().join("workspace"))
        .action_dir(second_root.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("resumed session host");
    resumed.set_thread_id(Some("legacy-history-lazy"));
    assert!(resumed.bound_history_is_authoritative().unwrap());
    assert!(!resumed
        .adopt_legacy_history_if_needed(|| async {
            Err(anyhow::anyhow!("legacy store unavailable"))
        })
        .await
        .unwrap());
    assert_eq!(
        resumed.turn("after restart").await.unwrap(),
        "reply after restart"
    );
    let resumed_request = second_model
        .requests()
        .into_iter()
        .next()
        .expect("resumed model request");
    let resumed_messages = serde_json::to_string(&resumed_request.messages).unwrap();
    assert!(resumed_messages.contains("legacy question"));
    assert!(resumed_messages.contains("first durable turn"));
    assert!(resumed_messages.contains("after restart"));
}
