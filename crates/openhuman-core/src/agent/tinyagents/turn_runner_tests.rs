use super::*;
use crate::agent::tinyagents::TurnModelSource;
use std::sync::Arc;

fn hosted_base() -> Arc<crate::agent::tinyagents::host::OpenHumanHostBase> {
    Arc::new(crate::agent::tinyagents::host::OpenHumanHostBase {
        config: Arc::new(crate::config::Config::default()),
        definitions: Arc::new(
            crate::agent::harness::definition::AgentDefinitionRegistry::builtins_only(),
        ),
        security_policy: Arc::new(crate::security::policy::SecurityPolicy::default()),
        memory: crate::memory::test_support::noop_memory(),
        post_turn_hooks: Vec::new(),
    })
}

fn root_models(reply: &str) -> TurnModels {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            reply,
        ]));
    TurnModelSource::from_model(model)
        .build("root-test-model", 0.0, None, None)
        .expect("scripted turn models build")
}

fn root_messages(label: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(format!("system-{label}")),
        ChatMessage::user(format!("user-{label}")),
    ]
}

fn root_context(
    thread_id: &str,
    workspace: &str,
    progress: tokio::sync::mpsc::Sender<crate::agent::progress::AgentProgress>,
) -> OpenHumanRunContext {
    let mut context = OpenHumanRunContext::new();
    context.origin = Some(crate::agent::turn_origin::AgentTurnOrigin::WebChat {
        thread_id: thread_id.to_string(),
        client_id: format!("client-{thread_id}"),
        request_id: Some(format!("request-{thread_id}")),
    });
    context.thread_id = Some(thread_id.to_string());
    context.workspace = Some(tinytools::WorkspaceDescriptor::new(workspace));
    context.progress = Some(progress);
    context
}

async fn run_root(
    base: Arc<crate::agent::tinyagents::host::OpenHumanHostBase>,
    context: OpenHumanRunContext,
    reply: &str,
) -> TinyagentsTurnOutcome {
    run_root_turn_via_hosted_agent(
        context,
        base,
        "main".to_string(),
        root_models(reply),
        "test".to_string(),
        "root-test-model",
        root_messages(reply),
        vec![Arc::new(Vec::new())],
        Some(Default::default()),
        2,
        None,
        None,
        &[],
        false,
        None,
        TurnContextMiddleware::default(),
        None,
        true,
    )
    .await
    .expect("hosted root succeeds")
}

#[tokio::test]
async fn precomposed_root_context_does_not_duplicate_the_session_prompt_or_preamble() {
    let request =
        TurnContextRequest::new("main", tinyagents_harness::ids::ThreadId::new("t"), "hi");

    assert_eq!(
        PrecomposedRootContext
            .compose_system_prompt(&request)
            .await
            .expect("precomposed root context composes"),
        "",
        "the frozen session system/context ladder stays in the invocation request"
    );
    assert!(
        PrecomposedRootContext
            .preamble(&request)
            .await
            .expect("precomposed root context builds preamble")
            .is_empty(),
        "host preparation must not insert a second root preamble"
    );
}

#[test]
fn hosted_roots_share_only_an_unconfigured_process_harness() {
    let first = root_hosted_harness() as *const _;
    let second = root_hosted_harness() as *const _;

    assert_eq!(first, second, "all roots enter the same durable harness");
    assert!(
        root_hosted_harness().models().default_name().is_none(),
        "models are invocation-local overlays, never mutable shared root state"
    );
    assert!(
        root_hosted_harness().tools().names().is_empty(),
        "tools are invocation-local overlays, never mutable shared root state"
    );
}

#[tokio::test]
async fn concurrent_hosted_roots_keep_models_progress_workspace_and_origin_isolated() {
    let base = hosted_base();
    let (left_progress, mut left_events) = tokio::sync::mpsc::channel(32);
    let (right_progress, mut right_events) = tokio::sync::mpsc::channel(32);

    let (left, right) = tokio::join!(
        run_root(
            base.clone(),
            root_context("left", "/tmp/left", left_progress),
            "left"
        ),
        run_root(
            base,
            root_context("right", "/tmp/right", right_progress),
            "right"
        ),
    );

    assert_eq!(left.text, "left");
    assert_eq!(right.text, "right");
    let left_history: Vec<_> = left
        .history
        .iter()
        .map(|message| (&message.role, &message.content))
        .collect();
    let right_history: Vec<_> = right
        .history
        .iter()
        .map(|message| (&message.role, &message.content))
        .collect();
    assert_ne!(
        left_history, right_history,
        "each overlay kept its transcript"
    );
    assert!(
        left_events.try_recv().is_ok(),
        "the left invocation retained its own progress sink"
    );
    assert!(
        right_events.try_recv().is_ok(),
        "the right invocation retained its own progress sink"
    );
}
