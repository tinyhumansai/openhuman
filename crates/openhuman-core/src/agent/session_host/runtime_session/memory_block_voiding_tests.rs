//! A render voided by a memory change, or overtaken by a newer one, does not
//! reach a later prompt; and the job, prewarm and account paths that drop
//! waiting blocks.

use super::invalidation::{a_block_waits, credential_config, first_turn_block, prewarmed};
use super::*;
use crate::agent::session_host::runtime_session::memory_block::NewGenerationBlock;

/// A first turn's refresh of a recorded block, with `forget` run `after` into
/// the render; the memory block the next prefix holds, if any.
async fn refreshed_across_a_change(config: &Config, after: std::time::Duration) -> Vec<String> {
    let prelude = prelude(config, "You are blocky, v2.", true);
    let (recorded, history) = generation_start();
    let change = async {
        tokio::time::sleep(after).await;
        crate::memory::lifecycle::prefetch::invalidate(config);
    };
    let view = view(&recorded, &history, None);
    let (prepared, ()) = tokio::join!(prepare(&prelude, false, &view), change);
    prepared
        .0
        .prefix
        .map(|prefix| prefix.messages().iter().map(Message::text).collect())
        .unwrap_or_default()
}

#[tokio::test(start_paused = true)]
async fn a_render_voided_without_a_deadline_does_not_restore_the_recorded_block() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    SlowEngine::bind(&SlowEngine::new(SLOW), &config);

    let texts = refreshed_across_a_change(&config, SLOW / 2).await;

    assert!(
        !texts.iter().any(|text| text.contains("teal")),
        "the block recorded before the change must not stand in: {texts:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_render_voided_inside_the_deadline_does_not_restore_the_recorded_block() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = with_deadline(config_in(&tmp), 60_000);
    SlowEngine::bind(&SlowEngine::new(SLOW), &config);

    let texts = refreshed_across_a_change(&config, SLOW / 2).await;

    assert!(!texts.iter().any(|text| text.contains("teal")), "{texts:?}");
}

#[tokio::test(start_paused = true)]
async fn a_failed_render_still_keeps_the_recorded_block() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::with(tinymemory_api::Error::Engine(
        "down".into(),
    ))
    .bind(&config);
    let prelude = prelude(&config, "You are blocky, v2.", true);
    let (recorded, history) = generation_start();

    let (prepared, _) = prepare(&prelude, false, &view(&recorded, &history, None)).await;

    let texts: Vec<String> = prepared
        .prefix
        .map(|prefix| prefix.messages().iter().map(Message::text).collect())
        .unwrap_or_default();
    assert!(texts.iter().any(|text| text.contains("teal")), "{texts:?}");
}

#[tokio::test(start_paused = true)]
async fn a_prewarm_voided_while_it_read_reports_that_nothing_waits() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    SlowEngine::bind(&SlowEngine::new(SLOW), &config);
    let change = async {
        tokio::time::sleep(SLOW / 2).await;
        crate::memory::lifecycle::prefetch::invalidate(&config);
    };

    let session = host(&config, PROMPT, false);
    let (prewarm, ()) = tokio::join!(session.prewarm_memory_block(), change);

    assert_eq!(prewarm, MemoryBlockPrewarm::Failed);
    assert!(!a_block_waits(&config).await, "nothing was stored");
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_job_run_drops_the_waiting_block() {
    use crate::memory::lifecycle::jobs;
    use tinymemory_api::{ConsolidateRequest, ItemKind, Namespace, Reach};
    use tinymemory_tools::queue::Selection;
    use tinymemory_tools::BackgroundJob;

    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.recall.build_delay_secs = 0;
    prewarmed(&config).await;
    // The engine now takes a while to build beliefs.
    SlowEngine::bind(&SlowEngine::new(SLOW), &config);
    jobs::enqueue(
        &config,
        &Namespace::ROOT,
        vec![BackgroundJob::BuildBeliefs {
            request: ConsolidateRequest::new(Reach::exact(Namespace::agent("a")))
                .kinds([ItemKind::Conversation]),
        }],
    )
    .await;

    // A profile fence cancels the run part way.
    let run = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        jobs::run(&config, Selection::Due),
    )
    .await;
    assert!(run.is_err(), "the run was cancelled");

    assert!(!a_block_waits(&config).await);
}

#[tokio::test(start_paused = true)]
async fn a_job_run_with_nothing_due_keeps_the_waiting_block() {
    use crate::memory::lifecycle::jobs;
    use tinymemory_tools::queue::Selection;

    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    prewarmed(&config).await;

    let runs = jobs::run(&config, Selection::Due).await.expect("ran");

    assert!(runs.is_empty());
    assert!(a_block_waits(&config).await);
}

#[tokio::test]
async fn a_provider_qualified_memory_credential_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    prewarmed(&config).await;

    crate::credentials::ops::store_provider_credentials(
        &config,
        "provider:memory-cortexdb",
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
async fn selecting_another_memory_account_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    let auth = crate::credentials::AuthService::from_config(&config);
    auth.store_provider_token(
        crate::config::schema::MEMORY_CORTEXDB_KEY_NAME,
        "second",
        "cdb-second",
        std::collections::HashMap::new(),
        false,
    )
    .await
    .expect("second account stored");
    prewarmed(&config).await;

    auth.set_active_profile(crate::config::schema::MEMORY_CORTEXDB_KEY_NAME, "second")
        .await
        .expect("selected");

    assert!(!a_block_waits(&config).await);
}

#[tokio::test]
async fn selecting_another_providers_profile_keeps_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    // Nothing else may clear the block meanwhile: an unrelated provider
    // never takes the read guard this holds out.
    let _stable = undisturbed();
    let auth = crate::credentials::AuthService::from_config(&config);
    auth.store_provider_token(
        "openai",
        "second",
        "sk-second",
        std::collections::HashMap::new(),
        false,
    )
    .await
    .expect("stored");
    prewarmed(&config).await;

    auth.set_active_profile("openai", "second")
        .await
        .expect("selected");

    assert!(a_block_waits(&config).await);
}

#[tokio::test(start_paused = true)]
async fn a_render_voided_while_the_turn_waits_does_not_restore_the_recorded_block_at_the_deadline()
{
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    // The turn gives up at 1s, with the render still reading until 5s.
    let config = with_deadline(config_in(&tmp), 1_000);
    SlowEngine::bind(&SlowEngine::new(SLOW), &config);

    let texts = refreshed_across_a_change(&config, std::time::Duration::from_millis(500)).await;

    assert!(!texts.iter().any(|text| text.contains("teal")), "{texts:?}");
}

#[tokio::test(start_paused = true)]
async fn an_older_prewarm_does_not_store_over_a_render_taken_without_a_deadline() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    // The prewarm starts first on a slow engine...
    SlowEngine::bind(&SlowEngine::new(SLOW), &config);
    let session = host(&config, PROMPT, false);
    let prewarm = session.prewarm_memory_block();
    // ...a generation start renders after it, on a quick one, and takes its
    // block directly.
    let start = async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        SlowEngine::bind(
            &SlowEngine::new(std::time::Duration::from_millis(100)),
            &config,
        );
        first_turn_block(&config).await
    };
    let (prewarm, _) = tokio::join!(prewarm, start);
    assert_eq!(
        prewarm,
        MemoryBlockPrewarm::Failed,
        "its older block was rejected"
    );

    assert!(
        !a_block_waits(&config).await,
        "the older prewarm stored its block over the newer render's"
    );
}

#[tokio::test(start_paused = true)]
async fn a_compaction_render_voided_by_an_erase_drops_the_inherited_block() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    SlowEngine::bind(&SlowEngine::new(SLOW), &config);
    let session = host(&config, PROMPT, false);
    let change = async {
        tokio::time::sleep(SLOW / 2).await;
        crate::memory::lifecycle::prefetch::invalidate(&config);
    };

    let (block, ()) = tokio::join!(session.new_generation_block(), change);

    assert!(
        matches!(block, NewGenerationBlock::Drop),
        "the inherited block may hold what was erased"
    );
}

#[tokio::test(start_paused = true)]
async fn a_compaction_render_nothing_changed_under_is_taken() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = SlowEngine::new(std::time::Duration::from_millis(10));
    SlowEngine::bind(&engine, &config);
    learn(engine.inner(), &config, "The user lives in Lisbon").await;

    let block = host(&config, PROMPT, false).new_generation_block().await;

    assert!(matches!(block, NewGenerationBlock::Fresh(_)));
}

#[tokio::test]
async fn replacing_the_memory_key_through_the_auth_service_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    prewarmed(&config).await;

    crate::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::config::schema::MEMORY_CORTEXDB_KEY_NAME,
            "default",
            "cdb-replaced",
            std::collections::HashMap::new(),
            true,
        )
        .await
        .expect("stored");

    assert!(!a_block_waits(&config).await);
}

#[tokio::test]
async fn removing_the_memory_key_through_the_auth_service_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    let auth = crate::credentials::AuthService::from_config(&config);
    auth.store_provider_token(
        crate::config::schema::MEMORY_CORTEXDB_KEY_NAME,
        "default",
        "cdb-key",
        std::collections::HashMap::new(),
        true,
    )
    .await
    .expect("stored");
    prewarmed(&config).await;

    assert!(auth
        .remove_profile(crate::config::schema::MEMORY_CORTEXDB_KEY_NAME, "default")
        .await
        .expect("removed"));

    assert!(!a_block_waits(&config).await);
}
