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

/// Whether a block is waiting for this session's next generation start: a
/// prewarm reads nothing when one is.
async fn a_block_waits(config: &Config) -> bool {
    host(config, PROMPT, false).prewarm_memory_block().await == MemoryBlockPrewarm::AlreadyReady
}

/// What tests that store credentials run under. The security module those go
/// through admits one secret scope per process, so they share one config
/// directory and run one at a time (CI runs each test in its own process).
/// They run in real time too: that module times out under a paused clock.
async fn credential_config() -> (tokio::sync::MutexGuard<'static, ()>, Config) {
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let serial = SERIAL.lock().await;
    let config = config_in(DIR.get_or_init(|| tempfile::tempdir().expect("config dir")));
    // Nothing a test before this one left waiting.
    crate::memory::lifecycle::prefetch::invalidate(&config);
    (serial, config)
}

/// A bound engine holding one fact, with a block prewarmed from it.
async fn prewarmed(config: &Config) -> Arc<SlowEngine> {
    let engine = SlowEngine::new(std::time::Duration::from_millis(10));
    SlowEngine::bind(&engine, config);
    learn(engine.inner(), config, "The user lives in Lisbon").await;
    assert_eq!(
        host(config, PROMPT, false).prewarm_memory_block().await,
        MemoryBlockPrewarm::Ready
    );
    assert!(a_block_waits(config).await);
    engine
}

#[tokio::test(start_paused = true)]
async fn a_render_inside_the_foreground_deadline_is_not_cut_at_the_background_limit() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = with_deadline(config_in(&tmp), 120_000);
    let engine = SlowEngine::new(std::time::Duration::from_secs(90));
    SlowEngine::bind(&engine, &config);
    learn(engine.inner(), &config, "The user lives in Lisbon").await;

    let block = first_turn_block(&config).await;

    assert!(
        block.unwrap_or_default().contains("Lisbon"),
        "a render the turn is still waiting for is not a background one"
    );
}

#[tokio::test]
async fn storing_the_memory_credential_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    let _stable = undisturbed();
    prewarmed(&config).await;

    crate::credentials::ops::store_provider_credentials(
        &config,
        crate::config::schema::MEMORY_CORTEXDB_KEY_NAME,
        None,
        Some("cdb-key".into()),
        None,
        None,
    )
    .await
    .expect("stored");

    assert!(!a_block_waits(&config).await);
}

#[tokio::test]
async fn removing_the_memory_credential_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    let _stable = undisturbed();
    prewarmed(&config).await;

    crate::credentials::ops::remove_provider_credentials(
        &config,
        crate::config::schema::MEMORY_CORTEXDB_KEY_NAME,
        None,
    )
    .await
    .expect("removed");

    assert!(!a_block_waits(&config).await);
}

#[tokio::test]
async fn another_providers_credential_keeps_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    let _stable = undisturbed();
    prewarmed(&config).await;

    crate::credentials::ops::store_provider_credentials(
        &config,
        "openai",
        None,
        Some("sk-other".into()),
        None,
        None,
    )
    .await
    .expect("stored");

    assert!(a_block_waits(&config).await);
}

#[tokio::test]
async fn storing_the_cortexdb_key_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    let _stable = undisturbed();
    prewarmed(&config).await;

    crate::memory::engine::store_cortexdb_key(&config, "cdb-key")
        .await
        .expect("stored");

    assert!(!a_block_waits(&config).await);
}

#[tokio::test]
async fn clearing_the_cortexdb_key_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    let _stable = undisturbed();
    crate::memory::engine::store_cortexdb_key(&config, "cdb-key")
        .await
        .expect("stored");
    prewarmed(&config).await;

    crate::memory::engine::clear_cortexdb_key(&config)
        .await
        .expect("cleared");

    assert!(!a_block_waits(&config).await);
}

/// Clears the host engine a test installed, even when it fails.
struct ClearHostEngine;

impl Drop for ClearHostEngine {
    fn drop(&mut self) {
        crate::memory::engine::clear_host_engine();
    }
}

#[tokio::test(start_paused = true)]
async fn replacing_the_host_engine_drops_the_waiting_block() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    prewarmed(&config).await;

    let _clear = ClearHostEngine;
    crate::memory::engine::install_host_engine(Arc::new(
        tinymemory_api::conformance::ReferenceEngine::new(),
    ));

    assert!(!a_block_waits(&config).await);
}

#[tokio::test(start_paused = true)]
async fn clearing_the_host_engine_drops_the_waiting_block() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    prewarmed(&config).await;

    crate::memory::engine::clear_host_engine();

    assert!(!a_block_waits(&config).await);
}
