use super::*;

#[tokio::test]
async fn without_a_backend_every_flow_is_local() {
    // The lib test binary never installs a backend into the process slot.
    if crate::storage::installed().is_some() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    assert_eq!(flow_owner(&config, "any-flow").await, Ok(None));
}

#[test]
fn a_forgotten_owner_is_resolved_again() {
    OWNERS
        .lock()
        .unwrap()
        .insert("owner-test-flow".to_string(), Some("agent-1".to_string()));
    forget("owner-test-flow");
    assert!(!OWNERS.lock().unwrap().contains_key("owner-test-flow"));
}

fn local_flow(id: &str) -> tinyflows_catalog::Flow {
    tinyflows_catalog::Flow {
        id: id.to_string(),
        name: id.to_string(),
        enabled: true,
        graph: tinyflows::model::WorkflowGraph::default(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_run_at: None,
        last_status: None,
        require_approval: false,
    }
}

#[tokio::test]
async fn a_cached_owner_is_dropped_once_its_scope_loses_the_flow() {
    if crate::storage::installed().is_some() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    crate::flows::store::upsert_flow(&config, &local_flow("owner-test-revalidate")).unwrap();

    // Found in `local`, and remembered there.
    assert_eq!(resolve(&config, "owner-test-revalidate").await, Ok(None));
    assert_eq!(
        OWNERS.lock().unwrap().get("owner-test-revalidate"),
        Some(&None)
    );
    // Still there: the cached answer stands.
    assert_eq!(resolve(&config, "owner-test-revalidate").await, Ok(None));
    assert!(OWNERS.lock().unwrap().contains_key("owner-test-revalidate"));

    // Gone: the cache entry is dropped and nothing is found.
    crate::flows::store::remove_flow(&config, "owner-test-revalidate").unwrap();
    assert_eq!(resolve(&config, "owner-test-revalidate").await, Ok(None));
    assert!(!OWNERS.lock().unwrap().contains_key("owner-test-revalidate"));
}
