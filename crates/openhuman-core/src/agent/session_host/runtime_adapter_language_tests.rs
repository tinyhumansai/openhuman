use super::*;

/// The reply-language instruction reaches the MODEL, on the user message next
/// to the clock line, and clearing it removes it from the next turn. Before
/// this, the web channel computed the directive, logged "injecting", and
/// dropped it: no request ever carried it.
#[test]
fn reply_language_directive_rides_each_user_message_until_cleared() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(crate::security::prompt_injection::with_scanner(
                    crate::security::prompt_injection::scanner_stub::StubScanner::phrases(),
                    reply_language_directive_rides_each_user_message_until_cleared_body(),
                ));
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

async fn reply_language_directive_rides_each_user_message_until_cleared_body() {
    const DIRECTIVE: &str = "User language: respond in Spanish (test directive).";
    let root = tempfile::tempdir().expect("tempdir");
    let model = Arc::new(tinyagents_harness::testkit::ScriptedModel::new(vec![
        tinyinference_llm::model::ModelResponse::assistant("hola"),
        tinyinference_llm::model::ModelResponse::assistant("hello"),
    ]));
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model.clone() as Arc<dyn tinyinference_llm::model::ChatModel<()>>)
        .model_name("test/session-fixture".to_string())
        .tools(Vec::new())
        .workspace_dir(root.path().join("workspace"))
        .action_dir(root.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .expect("session build");
    host.set_thread_id(Some("thread-reply-language"));
    let last_user_text = |request: &tinyinference_llm::model::ModelRequest| {
        request
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m, Message::User(_)))
            .map(|m| m.text())
            .expect("a user message reached the model")
    };

    host.set_reply_language_directive(Some(DIRECTIVE.to_string()));
    host.set_time_zone(Some("Pacific/Chatham".to_string()));
    assert_eq!(host.turn("¿qué tal?").await.unwrap(), "hola");
    host.set_reply_language_directive(None);
    assert_eq!(host.turn("and now?").await.unwrap(), "hello");

    let requests = model.requests();
    assert_eq!(requests.len(), 2, "one model call per turn");
    let first = last_user_text(&requests[0]);
    assert!(
        first.contains("Current Date & Time:"),
        "fixture: the clock line is there: {first}"
    );
    assert!(
        first.contains(DIRECTIVE),
        "the directive must reach the model: {first}"
    );
    assert!(
        first.contains(" Pacific/Chatham ("),
        "the clock reads in the user's zone: {first}"
    );
    assert!(
        first.find(DIRECTIVE) < first.find("¿qué tal?"),
        "the directive leads the user's own words: {first}"
    );
    let second = last_user_text(&requests[1]);
    assert!(
        second.contains("and now?"),
        "fixture: the second turn's text: {second}"
    );
    assert!(
        !second.contains(DIRECTIVE),
        "a cleared directive is gone next turn: {second}"
    );
}
