//! A memory block read before memory changed does not reach a later prompt.

use super::*;
use crate::agent::session_host::runtime_session::memory_block_ready::MAX_BACKGROUND_RENDERS;

/// The text of the memory block a new session's first turn is sent, if any.
pub(super) async fn first_turn_block(config: &Config) -> Option<String> {
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
pub(super) async fn a_block_waits(config: &Config) -> bool {
    host(config, PROMPT, false).prewarm_memory_block().await == MemoryBlockPrewarm::AlreadyReady
}

/// What tests that store credentials run under. The security module those go
/// through admits one secret scope per process, so they share one config
/// directory and run one at a time (CI runs each test in its own process).
/// They run in real time too: that module times out under a paused clock.
pub(super) async fn credential_config() -> (tokio::sync::MutexGuard<'static, ()>, Config) {
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let serial = SERIAL.lock().await;
    let config = config_in(DIR.get_or_init(|| tempfile::tempdir().expect("config dir")));
    // Nothing a test before this one left waiting.
    crate::memory::lifecycle::prefetch::invalidate(&config);
    (serial, config)
}

/// A bound engine holding one fact, with a block prewarmed from it.
pub(super) async fn prewarmed(config: &Config) -> Arc<SlowEngine> {
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
    // Nothing else may clear the block meanwhile: an unrelated credential
    // never takes the read guard this holds out.
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
    prewarmed(&config).await;

    crate::memory::engine::store_cortexdb_key(&config, "cdb-key")
        .await
        .expect("stored");

    assert!(!a_block_waits(&config).await);
}

#[tokio::test]
async fn clearing_the_cortexdb_key_drops_the_waiting_block() {
    let (_serial, config) = credential_config().await;
    crate::memory::engine::store_cortexdb_key(&config, "cdb-key")
        .await
        .expect("stored");
    prewarmed(&config).await;

    crate::memory::engine::clear_cortexdb_key(&config)
        .await
        .expect("cleared");

    assert!(!a_block_waits(&config).await);
}

/// Runs a test that installs the host engine in a child process of its own.
/// The host engine is the one every workspace resolves to, so installing it
/// here would change what unrelated tests in this process read. Returns
/// `true` in the child, which runs the body; the parent asserts the child
/// passed and returns `false`.
fn in_child_process(name: &str) -> bool {
    const CHILD_FLAG: &str = "OPENHUMAN_MEMORY_BLOCK_HOST_ENGINE_CHILD";
    if std::env::var_os(CHILD_FLAG).as_deref() == Some(std::ffi::OsStr::new("1")) {
        return true;
    }
    let path = module_path!().split_once("::").map_or("", |(_, rest)| rest);
    let exact = format!("{path}::{name}");
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &exact, "--nocapture"])
        .env(CHILD_FLAG, "1")
        .output()
        .expect("test child starts");
    assert!(
        child.status.success(),
        "host-engine child failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(
        String::from_utf8_lossy(&child.stdout).contains("1 passed"),
        "the exact child test must run: {}",
        String::from_utf8_lossy(&child.stdout)
    );
    false
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
    if !in_child_process("replacing_the_host_engine_drops_the_waiting_block") {
        return;
    }
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

#[tokio::test(start_paused = true)]
async fn prewarms_beyond_the_render_limit_are_not_started() {
    let _stable = undisturbed();
    let tmps = (0..=MAX_BACKGROUND_RENDERS)
        .map(|_| tempfile::tempdir().unwrap())
        .collect::<Vec<_>>();
    let configs = tmps.iter().map(config_in).collect::<Vec<_>>();
    for config in &configs {
        SlowEngine::bind(&SlowEngine::new(SLOW), config);
    }
    let hosts = configs
        .iter()
        .map(|config| host(config, PROMPT, false))
        .collect::<Vec<_>>();

    let results = futures::future::join_all(hosts.iter().map(|h| h.prewarm_memory_block())).await;

    let ready = results
        .iter()
        .filter(|r| **r == MemoryBlockPrewarm::Ready)
        .count();
    assert_eq!(ready, MAX_BACKGROUND_RENDERS, "{results:?}");
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == MemoryBlockPrewarm::Failed)
            .count(),
        1,
        "{results:?}"
    );

    // A finished prewarm freed its slot.
    let last = host(&configs[MAX_BACKGROUND_RENDERS], PROMPT, false);
    let again = last.prewarm_memory_block().await;
    assert!(
        matches!(
            again,
            MemoryBlockPrewarm::Ready | MemoryBlockPrewarm::AlreadyReady
        ),
        "{again:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_prewarm_is_abandoned_at_the_background_deadline() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = SlowEngine::new(std::time::Duration::from_secs(70));
    SlowEngine::bind(&engine, &config);

    assert_eq!(
        host(&config, PROMPT, false).prewarm_memory_block().await,
        MemoryBlockPrewarm::Failed
    );
    assert!(!a_block_waits(&config).await, "nothing was stored");
}

#[tokio::test(start_paused = true)]
async fn generation_starts_beyond_the_render_limit_keep_no_background_render() {
    let _stable = undisturbed();
    let tmps = (0..=MAX_BACKGROUND_RENDERS)
        .map(|_| tempfile::tempdir().unwrap())
        .collect::<Vec<_>>();
    let configs = tmps
        .iter()
        .map(|tmp| with_deadline(config_in(tmp), 300))
        .collect::<Vec<_>>();
    for config in &configs {
        SlowEngine::bind(&SlowEngine::new(SLOW), config);
    }
    let preludes = configs
        .iter()
        .map(|config| prelude(config, "You are blocky, v2.", true))
        .collect::<Vec<_>>();
    let (recorded, history) = generation_start();

    // Every start lapses its deadline and its render goes on in the
    // background, but only eight are admitted there.
    let (recorded, history) = (&recorded, &history);
    futures::future::join_all(preludes.iter().map(|p| async move {
        prepare(p, false, &view(recorded, history, None)).await;
    }))
    .await;
    tokio::time::sleep(SLOW * 2).await;

    let mut waiting = 0;
    for config in &configs {
        waiting += usize::from(a_block_waits(config).await);
    }
    assert_eq!(waiting, MAX_BACKGROUND_RENDERS);
}

#[tokio::test(start_paused = true)]
async fn a_write_through_a_shared_engine_drops_every_workspaces_block() {
    let _stable = undisturbed();
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let (config_one, config_two) = (config_in(&one), config_in(&two));
    prewarmed(&config_one).await;
    prewarmed(&config_two).await;

    // What `invalidate` runs while the host engine serves every workspace.
    crate::memory::lifecycle::prefetch::invalidate_with(&config_one, true);

    assert!(!a_block_waits(&config_one).await);
    assert!(!a_block_waits(&config_two).await);
}

#[tokio::test(start_paused = true)]
async fn another_workspaces_write_keeps_a_block_when_engines_are_not_shared() {
    let _stable = undisturbed();
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let (config_one, config_two) = (config_in(&one), config_in(&two));
    prewarmed(&config_one).await;
    prewarmed(&config_two).await;

    crate::memory::lifecycle::prefetch::invalidate_with(&config_one, false);

    assert!(!a_block_waits(&config_one).await);
    assert!(a_block_waits(&config_two).await);
}

#[tokio::test(start_paused = true)]
async fn a_background_memory_job_run_drops_the_waiting_block() {
    use crate::memory::lifecycle::jobs;
    use tinymemory_api::{ConsolidateRequest, ItemKind, Namespace, Reach};
    use tinymemory_tools::queue::Selection;
    use tinymemory_tools::BackgroundJob;

    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.recall.build_delay_secs = 0;
    prewarmed(&config).await;
    jobs::enqueue(
        &config,
        &Namespace::ROOT,
        vec![BackgroundJob::BuildBeliefs {
            request: ConsolidateRequest::new(Reach::exact(Namespace::agent("a")))
                .kinds([ItemKind::Conversation]),
        }],
    )
    .await;

    let runs = jobs::run(&config, Selection::Due).await.expect("ran");

    assert!(!runs.is_empty());
    assert!(!a_block_waits(&config).await);
}

#[tokio::test(start_paused = true)]
async fn a_render_whose_turn_was_cancelled_is_held_to_the_background_deadline() {
    let _stable = undisturbed();
    let tmp = tempfile::tempdir().unwrap();
    let config = with_deadline(config_in(&tmp), 120_000);
    SlowEngine::bind(
        &SlowEngine::new(std::time::Duration::from_secs(70)),
        &config,
    );
    let prelude = prelude(&config, "You are blocky, v2.", true);
    let (recorded, history) = generation_start();

    // The turn gives up long before its deadline.
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        prepare(&prelude, false, &view(&recorded, &history, None)),
    )
    .await;
    assert!(cancelled.is_err());
    tokio::time::sleep(std::time::Duration::from_secs(100)).await;

    assert!(
        !a_block_waits(&config).await,
        "the render ran on past the background limit"
    );
}
