use super::*;

use tinyagents_harness::summarization::{checkpoint_message, SummaryPlacement};
use tinyinference_llm::ToolCall;

fn assistant_with_call(id: &str, name: &str) -> Message {
    let mut message = Message::assistant("");
    if let Message::Assistant(assistant) = &mut message {
        assistant
            .tool_calls
            .push(ToolCall::new(id, name, serde_json::json!({})));
    }
    message
}

#[test]
fn without_a_checkpoint_the_whole_thread_is_in_the_prompt() {
    let history = vec![
        Message::user("one"),
        Message::assistant("a"),
        Message::user("two"),
    ];
    assert_eq!(in_prompt_window(&history, 2, None), (0, false));
}

#[test]
fn after_a_checkpoint_the_window_starts_at_the_first_kept_turn() {
    let current = Message::user("five");
    let history = vec![
        Message::system("prompt"),
        checkpoint_message(SummaryPlacement::User, "earlier turns"),
        Message::user("three"),
        Message::assistant("c"),
        Message::user("four"),
        Message::assistant("d"),
        current.clone(),
    ];
    // Five turns committed before this one; turns 3 and 4 are verbatim, so
    // the window opens at turn 3, user index 6.
    assert_eq!(in_prompt_window(&history, 5, Some(&current)), (6, true));
    assert_eq!(
        in_prompt_window(&history[..6], 5, None),
        (6, true),
        "the request input is not counted whether or not it was appended"
    );
}

#[test]
fn committed_tool_calls_pair_each_call_with_its_result() {
    let history = vec![
        Message::user("earlier"),
        assistant_with_call("old", "ignored"),
        Message::user("find the file"),
        assistant_with_call("c1", "glob"),
        Message::tool("c1", "src/main.rs"),
        assistant_with_call("c2", "noop"),
        Message::assistant("Found it."),
    ];
    let calls = committed_tool_calls(&history);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "glob");
    assert_eq!(calls[0].id.as_deref(), Some("c1"));
    assert_eq!(calls[0].result.as_deref(), Some("src/main.rs"));
    assert_eq!(calls[1].result, None);
}

#[tokio::test]
async fn deferred_memory_commit_survives_the_turn_scope_without_pinning_the_profile() {
    use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
    use crate::profiles::{ProfileHost, ProfileId};
    use tinymemory_api::{
        LearningKind, ListRequest, MemoryEngine, MemoryMeta, MetaFilter, StoreItem,
    };

    let root = tempfile::tempdir().unwrap();
    let engine = Arc::new(tinymemory_api::conformance::ReferenceEngine::new());
    let capabilities = Arc::new(crate::profiles::capabilities::ProfileCapabilities::new(
        None,
        None,
        None,
        Default::default(),
        Vec::new(),
        Some(engine.clone()),
    ));
    let host = ProfileHost::new(
        SaasConfig::new(root.path()),
        CoreContext::for_test(DomainSet::full(), None),
    )
    .with_capabilities(Some(capabilities));
    let id = ProfileId::parse("deferred-memory").unwrap();
    host.provision(&id).await.unwrap();
    let profile = host.open(&id).await.unwrap();
    let captured =
        capture_profile_context(&DispatchCtx::new(profile.context().clone()), Some(&host));
    // The entire temporary turn context is gone before this callback is polled.
    let config = profile.config.clone();
    let work = async {
        assert_eq!(
            crate::core::runtime::current_tenant()
                .unwrap()
                .profile
                .as_deref(),
            Some(id.as_str())
        );
        let binding = crate::memory::engine::resolve(&config).engine().unwrap();
        binding
            .engine
            .store(StoreItem::learning(
                "delayed committed reply",
                LearningKind::Fact,
                0.9,
                MemoryMeta {
                    namespace: config.memory.root.as_ref().unwrap().parse().unwrap(),
                    ..Default::default()
                },
            ))
            .await
            .unwrap();
    };
    deferred_memory_work(captured.clone(), work).await;
    assert_eq!(
        engine
            .list(ListRequest::new(MetaFilter::default(), 10))
            .await
            .unwrap()
            .items
            .len(),
        1,
        "a still-hosted profile must persist its deferred commit after its turn ends"
    );
    drop(profile);
    assert!(
        host.release(&id).await.unwrap(),
        "the pending weak capture cannot pin an idle profile"
    );
    assert!(
        matches!(&captured, ProfileMemoryContext::BoundWeak(context) if context.upgrade().is_none()),
        "the retired owner is not retained"
    );
    deferred_memory_work(captured, async {
        panic!("retired callbacks must fail closed")
    })
    .await;
}

#[tokio::test]
async fn missing_profile_owner_refuses_deferred_memory_instead_of_running_unscoped() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet, SaasConfig};
    use crate::profiles::ProfileHost;
    use crate::tools::toolpacks::ToolGroups;
    let root = tempfile::tempdir().unwrap();
    let operator = CoreContext::for_test(DomainSet::full(), None);
    let host = ProfileHost::new(SaasConfig::new(root.path()), operator.clone());
    let missing = operator.derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            ToolGroups::none(),
        )
        .profile("missing-owner"),
    );
    let captured = capture_profile_context(&DispatchCtx::new(missing), Some(&host));
    deferred_memory_work(captured, async {
        panic!("a failed profile owner capture must not run under the configured external engine")
    })
    .await;
}

#[tokio::test]
async fn failed_profile_capture_is_denied_but_ordinary_desktop_work_still_runs() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    let operator = CoreContext::for_test(DomainSet::full(), None);
    for id in ["missing-host", "invalid/owner"] {
        let profile = operator.derive_with(
            ContextOverlay::new(
                crate::config::Config::default(),
                DomainSet::full(),
                ToolGroups::none(),
            )
            .profile(id),
        );
        let captured = capture_profile_context(&DispatchCtx::new(profile), None);
        assert!(matches!(captured, ProfileMemoryContext::Denied));
        deferred_memory_work(captured, async {
            panic!("a profile without its host cannot resolve an external engine")
        })
        .await;
    }
    let captured = capture_profile_context(&DispatchCtx::new(operator), None);
    assert!(matches!(captured, ProfileMemoryContext::NonProfile));
    let mut ran = false;
    deferred_memory_work(captured, async {
        ran = true;
    })
    .await;
    assert!(
        ran,
        "ordinary desktop work retains its existing deferred behavior"
    );
}
