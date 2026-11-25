use super::*;
use crate::agent_registry::host_definition::MemoryBlock;
use crate::config::schema::EphemeralRoute;
use crate::memory::test_fixtures::bind_reference;
use tinymemory_api::{LearningKind, MemoryEngine, MemoryMeta, StoreItem};

const MODEL: &str = "turn-model";

/// A memory-block agent definition and a config whose memory holds one fact.
async fn memory_block_agent(
    tmp: &tempfile::TempDir,
) -> (
    Config,
    crate::agent_registry::host_definition::AgentDefinition,
) {
    // Canonical, because the secrets store refuses a path through a symlink
    // (macOS's temp directory is one).
    let root = tmp.path().canonicalize().expect("canonical temp dir");
    let mut config = Config {
        workspace_dir: root.join("workspace"),
        action_dir: root.join("workspace"),
        config_path: root.join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config.secrets.encrypt = false;
    // A signed-in app session, which any non-library host requires of a
    // provider that is not the caller's own.
    crate::credentials::api_key::store_api_key(&config, "test-session-key")
        .await
        .expect("store the session credential");
    let engine = bind_reference(&config);
    let (_, identity) = crate::memory::lifecycle::current_agent_memory(
        &config,
        &tinymemory_tools::layout::identity::Identity::root(),
    )
    .await
    .expect("memory identity");
    engine
        .store(StoreItem::learning(
            "The user lives in Lisbon",
            LearningKind::Fact,
            0.9,
            MemoryMeta {
                namespace: identity.layout.learnings().clone(),
                ..MemoryMeta::default()
            },
        ))
        .await
        .expect("stored");
    let mut definition = crate::agent::harness::test_support::test_main_def();
    definition.extensions.memory_block = Some(MemoryBlock { max_tokens: 500 });
    (config, definition)
}

fn target(
    definition: &crate::agent_registry::host_definition::AgentDefinition,
) -> AgentChatTarget<'_> {
    AgentChatTarget::Definition {
        definition,
        host: None,
        seed: None,
        usage: None,
        host_only: false,
        untrusted_input: false,
        shape: None,
        observer: None,
        attachments: &[],
    }
}

fn hosted_route() -> EphemeralRoute {
    EphemeralRoute {
        endpoint: "http://127.0.0.1:9/v1".into(),
        api_key: "sk-test".into(),
        ..EphemeralRoute::default()
    }
}

/// An embedder on a hosted route only has no managed provider to fall back
/// on: the prewarm must build its session with the turn's route and model, as
/// `compact_thread_for` does, or the build answers `BACKEND_UNAVAILABLE`.
#[tokio::test]
async fn prewarm_memory_block_uses_turn_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut config, definition) = memory_block_agent(&tmp).await;

    let prewarm = prewarm_memory_block_for(
        &mut config,
        target(&definition),
        "thread-prewarm",
        Some(MODEL.into()),
        None,
        Some(hosted_route()),
    )
    .await
    .expect("a session on the turn's route builds");

    assert_eq!(prewarm, MemoryBlockPrewarm::Ready);
}

/// Without the route the session cannot be built at all: the failure the
/// overrides above exist to avoid.
#[tokio::test]
async fn prewarm_memory_block_without_overrides_has_no_hosted_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut config, definition) = memory_block_agent(&tmp).await;

    let error = prewarm_memory_block_for(
        &mut config,
        target(&definition),
        "thread-prewarm",
        None,
        None,
        None,
    )
    .await
    .expect_err("no hosted provider is registered");

    assert!(error.to_string().contains("BACKEND_UNAVAILABLE"), "{error}");
}
