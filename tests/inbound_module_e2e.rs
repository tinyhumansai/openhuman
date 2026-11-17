//! Inbound queue admission through the verified TinyEventQueues release module.
//!
//! The staged release archive is digest-verified and loaded as a local file,
//! which is the supported source for modules that use host callbacks. The
//! host's storage backs its queues (an in-memory backend here), the host's
//! `Deliver` callback is the production one. The integration pins the host's
//! admission path; TinyEventQueues owns deduplication and delivery behavior.
//!
//! Run after staging test modules: `cargo test -p openhuman-cli --features inbound-queues,e2e-test-support --test inbound_module_e2e`

use openhuman_core::config::schema::ModuleOverride;
use openhuman_core::config::Config;
use openhuman_core::inbound::{events, route};
use openhuman_core::modules::types::{ModuleSource, ModuleState};
use openhuman_core::modules::{event_queues, ops};
use openhuman_core::triage::TriggerEnvelope;
use tinyeventqueues_bus::Disposition;

fn config() -> Config {
    let mut config = Config::default();
    config.modules.enabled = true;
    // A fallback to the release must fail loudly rather than download.
    config.modules.allow_download = false;
    let artifact = openhuman_core::modules::staged_artifact("tinyeventqueues")
        .expect("stage the verified TinyEventQueues release module");
    config.modules.overrides = vec![ModuleOverride::with_path(
        "tinyeventqueues",
        artifact.to_string_lossy().into_owned(),
    )];
    config.components.eventqueues = true;
    config.inbound.enabled = true;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tinyeventqueues_accepts_a_composio_trigger_into_storage() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    std::fs::write(
        workspace.path().join("config.toml"),
        "[inbound]\ntriage_timeout_ms = 1500\n",
    )
    .expect("seed config.toml");
    // This target runs one test in its own process; nothing else reads these.
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace.path());
    std::env::set_var("BACKEND_URL", "http://127.0.0.1:9");
    let previous = openhuman_core::storage::install(std::sync::Arc::new(
        tinystoragedrivers_testkit::MemoryBackend::new(),
    ));
    assert!(previous.is_none());
    let config = config();
    let client = ops::client(config.clone());
    let envelope = TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "msg-1",
        "uuid-linked-1",
        serde_json::json!({ "subject": "linked e2e" }),
    );

    // Ingest: the module loads from the executable and persists the event.
    let event = events::composio(&envelope).expect("a composio trigger is queueable");
    let first = route::offer(&config, &client, event.clone()).await;
    assert_eq!(first, route::Offered::Queued(Disposition::Accepted));
    let status = ops::list(&config)
        .into_iter()
        .find(|module| module.id == event_queues::MODULE_ID)
        .expect("tinyeventqueues is registered");
    assert_eq!(status.state, ModuleState::Ready);
    assert_eq!(status.source, Some(ModuleSource::LocalFile));

    // Accepted means the module persisted the ingress before returning. Its
    // timer-driven deduplication and delivery loop are covered in TinyEventQueues.
}
