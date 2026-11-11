use super::*;

use super::tests::{drain, HeldEngine, CACHED_NOTICE, TEST_LOCK};

use crate::memory::test_fixtures::{bind_reference, config_in};

async fn remember_teal(engine: &tinymemory_api::conformance::ReferenceEngine) {
    use tinymemory_api::MemoryEngine;
    engine
        .store(tinymemory_api::StoreItem::learning(
            "The user's favourite colour is teal",
            tinymemory_api::LearningKind::Preference,
            0.9,
            tinymemory_api::MemoryMeta::default(),
        ))
        .await
        .unwrap();
}

async fn turn_on(
    config: &Arc<Config>,
    identity: &ResolvedIdentity,
    thread: &str,
    turn_index: u32,
    each_turn: Option<RecallEachTurn>,
) -> Option<TurnPack> {
    pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        config.clone(),
        identity.clone(),
        thread.into(),
        turn_index,
        "what colour do I like?".into(),
        0,
        chrono::Utc::now(),
        false,
        None,
        None,
        each_turn,
    )
    .await
}

#[tokio::test]
async fn recall_each_turn_answers_the_current_message_within_its_deadline() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = Arc::new(config_in(&tmp));
    remember_teal(&bind_reference(&config)).await;
    let identity = tinymemory_tools::layout::identity::Identity::agent("each-turn")
        .resolve(&crate::memory::scope::identity_pins(&config));

    let pack = turn_on(
        &config,
        &identity,
        "each-turn",
        0,
        Some(RecallEachTurn::new(Duration::from_secs(5))),
    )
    .await
    .expect("the first turn already carries its own pack");
    assert!(pack.markdown.contains("teal"), "{}", pack.markdown);
    assert!(
        !pack.markdown.contains(CACHED_NOTICE.trim()),
        "a pack for this message, not a cached one from an earlier turn"
    );
    drain(
        &cache(&crate::core::runtime::DispatchCtx::ambient()),
        &Key::new(&config, &identity, "each-turn", false),
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn recall_each_turn_gives_up_at_its_deadline_and_adds_no_latency_beyond_it() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let engine = Arc::new(HeldEngine {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        release: release.clone(),
    });
    remember_teal(&engine.inner).await;
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    let config = Arc::new(config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("each-turn-late")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let deadline = Duration::from_millis(800);

    let started = tokio::time::Instant::now();
    let pack = turn_on(
        &config,
        &identity,
        "each-turn-late",
        0,
        Some(RecallEachTurn::new(deadline)),
    )
    .await;
    assert!(pack.is_none(), "a late recall adds no note");
    assert_eq!(
        started.elapsed(),
        deadline,
        "the turn waits the deadline, no longer"
    );

    release.add_permits(100);
    drain(
        &cache(&crate::core::runtime::DispatchCtx::ambient()),
        &Key::new(&config, &identity, "each-turn-late", false),
    )
    .await;
}

#[tokio::test]
async fn without_recall_each_turn_a_turn_still_reads_only_completed_packs() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = Arc::new(config_in(&tmp));
    remember_teal(&bind_reference(&config)).await;
    let identity = tinymemory_tools::layout::identity::Identity::agent("each-turn-off")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let cache = cache(&crate::core::runtime::DispatchCtx::ambient());
    let key = Key::new(&config, &identity, "each-turn-off", false);

    assert!(
        turn_on(&config, &identity, "each-turn-off", 0, None)
            .await
            .is_none(),
        "a cold cache gives the first turn nothing"
    );
    drain(&cache, &key).await;
    let pack = turn_on(&config, &identity, "each-turn-off", 2, None)
        .await
        .expect("the next turn reads the completed pack");
    assert!(
        pack.markdown.starts_with(CACHED_NOTICE),
        "{}",
        pack.markdown
    );
    drain(&cache, &key).await;
}

#[tokio::test]
async fn an_excluded_recall_leaves_the_turn_without_a_pack() {
    let _guard = TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let config = Arc::new(config_in(&tmp));
    remember_teal(&bind_reference(&config)).await;
    let identity = tinymemory_tools::layout::identity::Identity::agent("each-turn-excluded")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let cache = cache(&crate::core::runtime::DispatchCtx::ambient());
    let key = Key::new(&config, &identity, "each-turn-excluded", false);

    let unrelated = RecallEachTurn::new(Duration::from_secs(5)).excluding(Arc::new(
        |hit: &tinymemory_api::Hit| hit.text.contains("violet"),
    ));
    let kept = turn_on(&config, &identity, "each-turn-excluded", 0, Some(unrelated))
        .await
        .expect("a pack with nothing excluded is shown");
    assert!(kept.markdown.contains("teal"));
    drain(&cache, &key).await;

    let everything = RecallEachTurn::new(Duration::from_secs(5)).excluding(Arc::new(
        |hit: &tinymemory_api::Hit| hit.text.contains("teal"),
    ));
    assert!(
        turn_on(
            &config,
            &identity,
            "each-turn-excluded",
            2,
            Some(everything)
        )
        .await
        .is_none(),
        "a pack whose every cited hit is excluded is left out"
    );
    drain(&cache, &key).await;
}
