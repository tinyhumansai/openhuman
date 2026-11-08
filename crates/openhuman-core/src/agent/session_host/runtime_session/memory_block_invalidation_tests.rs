//! A memory block read before memory changed does not reach a later prompt.

use super::*;

/// The text of the memory block a new session's first turn is sent, if any.
async fn first_turn_block(config: &Config) -> Option<String> {
    let prelude = prelude(config, PROMPT, false);
    let empty = PrefixSnapshot::default();
    let (prepared, _) = prepare(&prelude, true, &view(&empty, &[], None)).await;
    sent(&prepared, &empty)
        .iter()
        .find(|m| is_memory_block(m))
        .map(Message::text)
}

#[tokio::test(start_paused = true)]
async fn a_forgotten_fact_is_not_in_the_next_turns_block_after_a_prewarm() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = SlowEngine::new(std::time::Duration::from_millis(10));
    SlowEngine::bind(&engine, &config);
    let id = learn(engine.inner(), &config, "The user lives in Lisbon").await;
    assert_eq!(
        host(&config, PROMPT, false).prewarm_memory_block().await,
        MemoryBlockPrewarm::Ready
    );

    crate::memory::ops::forget(
        &config,
        crate::memory::types::ForgetParams {
            ids: vec![id],
            reach: None,
        },
    )
    .await
    .expect("forgotten");

    let block = first_turn_block(&config).await;
    assert!(
        !block.unwrap_or_default().contains("Lisbon"),
        "the block read before the erase must not reach a new prompt"
    );
}

#[tokio::test(start_paused = true)]
async fn a_fact_learned_after_a_prewarm_is_in_the_next_turns_block() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = SlowEngine::new(std::time::Duration::from_millis(10));
    SlowEngine::bind(&engine, &config);
    assert_eq!(
        host(&config, PROMPT, false).prewarm_memory_block().await,
        MemoryBlockPrewarm::Ready,
        "nothing known yet: an empty block waits"
    );

    crate::memory::ops::learn(
        &config,
        crate::memory::types::LearnParams {
            text: "The user lives in Lisbon".into(),
            kind: None,
            confidence: None,
            meta: None,
        },
        None,
    )
    .await
    .expect("learned");

    let block = first_turn_block(&config).await;
    assert!(
        block.unwrap_or_default().contains("Lisbon"),
        "the empty block read before the write must not hide it"
    );
}

#[tokio::test(start_paused = true)]
async fn a_changed_memory_engine_drops_the_waiting_block() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = SlowEngine::new(std::time::Duration::from_millis(10));
    SlowEngine::bind(&engine, &config);
    learn(engine.inner(), &config, "The user lives in Lisbon").await;
    host(&config, PROMPT, false).prewarm_memory_block().await;
    let reads = engine.reads();

    // What a CortexDB endpoint or key change runs.
    crate::memory::engine::invalidate();

    first_turn_block(&config).await;
    assert!(
        engine.reads() > reads,
        "the turn read memory itself instead of taking the old block"
    );
}

#[tokio::test(start_paused = true)]
async fn a_render_in_flight_across_an_erase_is_not_kept_for_the_next_turn() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = with_deadline(config_in(&tmp), 300);
    let engine = SlowEngine::new(SLOW);
    SlowEngine::bind(&engine, &config);
    let id = learn(engine.inner(), &config, "The user lives in Lisbon").await;
    let prelude = prelude(&config, "You are blocky, v2.", true);
    let (recorded, history) = generation_start();

    // The render lapses its deadline and goes on in the background...
    prepare(&prelude, false, &view(&recorded, &history, None)).await;
    // ...and the fact is erased while it reads.
    crate::memory::ops::forget(
        &config,
        crate::memory::types::ForgetParams {
            ids: vec![id],
            reach: None,
        },
    )
    .await
    .expect("forgotten");
    tokio::time::sleep(SLOW).await;

    let reads = engine.reads();
    let (next, _) = prepare(&prelude, false, &view(&recorded, &history, None)).await;
    let next = next.prefix.expect("a refreshed prefix");
    assert!(
        !next.messages().iter().any(|m| m.text().contains("Lisbon")),
        "{next:?}"
    );
    assert!(engine.reads() > reads, "the stale render was not taken");
}

#[tokio::test(start_paused = true)]
async fn a_background_render_is_abandoned_at_its_deadline() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = with_deadline(config_in(&tmp), 300);
    let engine = SlowEngine::new(std::time::Duration::from_secs(70));
    SlowEngine::bind(&engine, &config);
    learn(engine.inner(), &config, "The user lives in Lisbon").await;
    let prelude = prelude(&config, "You are blocky, v2.", true);
    let (recorded, history) = generation_start();

    prepare(&prelude, false, &view(&recorded, &history, None)).await;
    tokio::time::sleep(std::time::Duration::from_secs(300)).await;

    let reads = engine.reads();
    let (next, _) = prepare(&prelude, false, &view(&recorded, &history, None)).await;
    let next = next.prefix.expect("a refreshed prefix");
    assert!(
        !next.messages().iter().any(|m| m.text().contains("Lisbon")),
        "a render past its deadline stored nothing"
    );
    assert!(engine.reads() > reads, "so the next start rendered again");
}
