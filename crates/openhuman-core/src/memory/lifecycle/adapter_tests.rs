use super::*;

use tinymemory_api::{Hit, ItemId, ItemKind, MemoryMeta};
use tinymemory_tools::lifecycle::turn::TurnPack as NativeTurnPack;

#[test]
fn native_pack_hits_convert_to_the_web_chat_citation_view() {
    let hit = |id: &str| Hit {
        id: ItemId(id.to_string()),
        kind: ItemKind::Learning,
        text: id.to_string(),
        meta: MemoryMeta::default(),
        score: 1.0,
        confidence: None,
    };
    let native = NativeTurnPack {
        markdown: "shown memory".into(),
        tokens: 2,
        refs: vec!["first".into(), "second".into()],
        engine: "test".into(),
        citations: vec![hit("first"), hit("second")],
        refusal: None,
    };

    let turn_pack = TurnPack::from_native(native);
    assert_eq!(turn_pack.refs, ["first", "second"]);
    assert_eq!(
        turn_pack
            .citations
            .iter()
            .map(|citation| citation.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[tokio::test]
async fn background_prefetch_records_citations_only_when_returning_the_pack() {
    let tmp = tempfile::tempdir().unwrap();
    let config = std::sync::Arc::new(config_in(&tmp));
    let engine = bind_reference(&config);
    engine
        .store(StoreItem::learning(
            "The preferred colour is teal",
            LearningKind::Preference,
            0.9,
            MemoryMeta::default(),
        ))
        .await
        .unwrap();
    let identity = tinymemory_tools::layout::identity::Identity::agent("citation-agent")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let ctx = crate::core::runtime::DispatchCtx::ambient();

    assert!(super::super::super::prefetch::pre_turn(
        &ctx,
        config.clone(),
        identity.clone(),
        "citation-thread".into(),
        0,
        "what colour is preferred?".into(),
        0,
        Utc::now(),
        false,
        None,
        None,
        None,
    )
    .await
    .is_none());
    assert!(crate::web_chat::citations::take_turn_citations(&ctx, "citation-thread").is_empty());

    super::super::super::prefetch::settle_for_test(
        &ctx,
        &config,
        &identity,
        "citation-thread",
        false,
    )
    .await;
    let pack = super::super::super::prefetch::pre_turn(
        &ctx,
        config,
        identity,
        "citation-thread".into(),
        2,
        "what colour is preferred?".into(),
        0,
        Utc::now(),
        false,
        None,
        None,
        None,
    )
    .await
    .expect("completed prefetch pack");
    assert!(
        pack.refusal.is_none(),
        "a refusal is not a shown citation pack"
    );
    assert!(!pack.citations.is_empty(), "the visible pack has citations");
    let recorded = crate::web_chat::citations::take_turn_citations(&ctx, "citation-thread");
    assert_eq!(
        recorded
            .iter()
            .map(|citation| citation.id.as_str())
            .collect::<Vec<_>>(),
        pack.citations
            .iter()
            .map(|citation| citation.id.as_str())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn replaying_a_committed_reply_does_not_queue_its_build_twice() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.recall.build_beliefs_every = 2;
    bind_reference(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("writer")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let committed = reply("replayed-thread", 1, "The reply is committed.");

    run_post_turn(&config, &identity, committed.clone()).await;
    let first = crate::memory::lifecycle::jobs::snapshot(&config)
        .await
        .pending;
    assert_eq!(first.len(), 1);

    run_post_turn(&config, &identity, committed).await;
    assert_eq!(
        crate::memory::lifecycle::jobs::snapshot(&config)
            .await
            .pending,
        first
    );
}

#[test]
fn memory_turn_keeps_its_core_context_without_owning_the_core() {
    let tmp = tempfile::tempdir().unwrap();
    let config = std::sync::Arc::new(config_in(&tmp));
    let identity = tinymemory_tools::layout::identity::Identity::agent("context-agent")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let core = crate::core::runtime::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(config.workspace_dir.clone()),
    );
    let weak = std::sync::Arc::downgrade(&core);
    let turn = MemoryTurn {
        config,
        identity,
        thread_id: "context-thread".into(),
        pack: None,
        in_prompt_from: std::sync::Arc::new(
            tinymemory_tools::lifecycle::window::InPromptFrom::new(2, 4),
        ),
        core: Some(weak.clone()),
    };

    assert_eq!(turn.in_prompt().from_turn, 2);
    assert!(std::sync::Arc::ptr_eq(
        turn.dispatch_ctx().core().unwrap(),
        &core
    ));
    assert_eq!(
        std::sync::Arc::strong_count(&core),
        1,
        "MemoryTurn stores only a weak core"
    );
    drop(core);
    assert!(
        weak.upgrade().is_none(),
        "the turn does not retain the core lease"
    );
}
