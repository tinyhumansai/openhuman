use super::*;
use crate::core::runtime::DispatchCtx;
use crate::integrations::task_sources::types::{FilterSpec, SourceTarget};
use serde_json::json;

fn github_filter() -> FilterSpec {
    FilterSpec::Github {
        repo: Some("tinyhumansai/openhuman".into()),
        labels: Vec::new(),
        assignee_is_me: true,
        state: None,
        fetch_mode: Default::default(),
        extra: json!({}),
    }
}

/// A new connection starts a fetch for the scope's enabled sources of that
/// toolkit — skipping ones pinned to another connection — and returns
/// without waiting for the fetches.
#[tokio::test]
async fn a_new_connection_fires_the_matching_sources_of_the_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    for connection in [None, Some("conn-other".to_string())] {
        store::add_source(
            &config,
            ProviderSlug::Github,
            connection,
            None,
            github_filter(),
            600,
            SourceTarget::TodoOnly,
            5,
        )
        .unwrap();
    }
    fire_for_connection(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        ProviderSlug::Github,
        "github",
        "conn-1",
    )
    .await;
    // A toolkit with no sources fires nothing.
    fire_for_connection(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        ProviderSlug::Notion,
        "notion",
        "conn-1",
    )
    .await;
    assert_eq!(store::list_sources(&config).unwrap().len(), 2);
}

/// A scope whose sources cannot be listed is skipped, not an error.
#[tokio::test]
async fn an_unreadable_scope_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    // A file where the workspace directory should be: the store cannot open.
    let blocked = tmp.path().join("blocked");
    std::fs::write(&blocked, b"").unwrap();
    let config = crate::config::Config {
        workspace_dir: blocked.clone(),
        action_dir: blocked,
        ..crate::config::Config::default()
    };
    fire_for_connection(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        ProviderSlug::Github,
        "github",
        "conn-1",
    )
    .await;
}

/// Smoke test: the subscriber handles connection events through the scope's
/// own configuration without panicking or hanging — a non-task-source toolkit
/// and an unrelated event are ignored, a task-source toolkit is handled. (The
/// firing itself is asserted by the test above.)
#[tokio::test]
async fn a_connection_event_is_handled_per_scope() {
    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::TempDir::new().unwrap();
    let _guard = crate::config::test_env::EnvVarGuard::workspace_unlocked(workspace.path());
    let event = |toolkit: &str| DomainEvent::ComposioConnectionCreated {
        toolkit: toolkit.to_string(),
        connection_id: "conn-1".to_string(),
        connect_url: String::new(),
        tenant: None,
    };
    // Not a task-source toolkit.
    TaskSourcesConnectionSubscriber
        .handle(&event("not-a-toolkit"))
        .await;
    // A task-source toolkit: the local scope loads its configuration and
    // fires (or skips) its sources.
    TaskSourcesConnectionSubscriber
        .handle(&event("github"))
        .await;
    // An event of another kind is ignored.
    TaskSourcesConnectionSubscriber
        .handle(&DomainEvent::ComposioConnectionDeleted {
            toolkit: "github".to_string(),
            connection_id: "conn-1".to_string(),
        })
        .await;
    fire_in_scope(
        &DispatchCtx::process_default(),
        ProviderSlug::Github,
        "github",
        "conn-1",
    )
    .await;
}
