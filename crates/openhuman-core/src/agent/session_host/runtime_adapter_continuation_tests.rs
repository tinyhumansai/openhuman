use super::*;

#[test]
fn a_cold_resumed_thread_can_send_again() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(cold_resumed_thread_can_send_again());
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

async fn cold_resumed_thread_can_send_again() {
    let root = tempfile::tempdir().expect("tempdir");
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(vec![
            tinyinference_llm::model::ModelResponse::assistant("first reply"),
            tinyinference_llm::model::ModelResponse::assistant("second reply"),
        ]));
    let new_host = || {
        crate::agent::SessionHostBuilder::new()
            .chat_model(model.clone())
            .model_name("test/session-fixture".to_string())
            .tools(Vec::new())
            .workspace_dir(root.path().join("workspace"))
            .action_dir(root.path().to_path_buf())
            .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
            .build()
            .expect("session build")
    };
    let mut host = new_host();
    host.set_thread_id(Some("thread-resume-regression"));

    assert_eq!(host.turn("first message").await.unwrap(), "first reply");
    drop(host);
    let mut host = new_host();
    host.set_thread_id(Some("thread-resume-regression"));
    assert_eq!(host.turn("second message").await.unwrap(), "second reply");
}

/// The PRODUCTION turn path must wire an artifact store, rooted outside the
/// project the agent works in (#6408).
///
/// Two assertions, and both were dead code before this PR. The existing artifact
/// tests construct `TurnContextMiddleware` with `artifact_store: Some(..)`
/// themselves, so they prove the offload code works while saying nothing about
/// whether production ever reaches it — and it did not: the production
/// constructor hard-coded `None`, which is how a whole feature shipped dead.
///
/// The root matters as much as the wiring, and it is the workspace's
/// `artifacts/tool-results`, never a directory the agent works in. Rooted at
/// `action_dir` (or a descriptor's root) every oversized output was a stray file
/// in the user's project, swept into its diff. The pointer is absolute, so it
/// no longer has to match whichever root `file_read` resolves relative paths
/// against (#6483). This drives a real host with a descriptor whose root is not
/// `action_dir`, and pins that the store followed neither.
#[tokio::test]
async fn production_turn_path_wires_an_artifact_store_outside_the_project() {
    let workspace = tempfile::tempdir().expect("workspace");
    let action_dir = tempfile::tempdir().expect("tempdir");
    let turn_root = tempfile::tempdir().expect("turn root");
    assert_ne!(
        action_dir.path(),
        turn_root.path(),
        "the two roots must differ or this test cannot tell them apart"
    );

    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .model_name("test/session-fixture".to_string())
        .tools(Vec::new())
        .workspace_dir(workspace.path().to_path_buf())
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .workspace_descriptor(Some(
            tinytools::WorkspaceDescriptor::new(turn_root.path().to_path_buf())
                .with_policy_id("turn-workspace"),
        ))
        .build()
        .expect("session build");

    host.ensure_runtime_session()
        .expect("ensure runtime session");

    let state = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let middleware = state
        .context_middleware
        .as_ref()
        .expect("the turn path must retain its context middleware");
    let store = middleware.artifact_store.as_ref().expect(
        "the production turn path must wire an artifact store, or oversized \
         tool results are truncated with their tail discarded (#6408)",
    );
    assert_eq!(
        store.materialize_root(),
        crate::security::policy::tool_result_artifacts_dir(workspace.path()),
        "the store must live under the workspace, not in the project the agent edits"
    );
    assert!(!store.materialize_root().starts_with(action_dir.path()));
    assert!(!store.materialize_root().starts_with(turn_root.path()));
}

/// Without a descriptor the answer is the same: the workspace, not
/// `action_dir`. The mirror of the test above, so neither branch can regress
/// back into the project.
#[tokio::test]
async fn artifact_store_stays_in_the_workspace_without_a_descriptor() {
    let workspace = tempfile::tempdir().expect("workspace");
    let action_dir = tempfile::tempdir().expect("tempdir");
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .model_name("test/session-fixture".to_string())
        .tools(Vec::new())
        .workspace_dir(workspace.path().to_path_buf())
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .expect("session build");

    host.ensure_runtime_session()
        .expect("ensure runtime session");

    let state = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let store = state
        .context_middleware
        .as_ref()
        .expect("context middleware")
        .artifact_store
        .as_ref()
        .expect("artifact store");
    assert_eq!(
        store.materialize_root(),
        crate::security::policy::tool_result_artifacts_dir(workspace.path())
    );
}

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
                .block_on(reply_language_directive_rides_each_user_message_until_cleared_body());
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
