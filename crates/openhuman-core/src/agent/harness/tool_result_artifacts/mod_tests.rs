use super::*;
use crate::security::SecurityPolicy;
use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::artifacts::tool_results::apply_per_result_persistence;
use tinytools::Tool;

/// End to end through OpenHuman's wiring: the real `sanitize_text` redacts the
/// stored body and the preview, the body lands on the blob store (nothing in
/// the project the agent is editing), and the real `file_read` opens it through
/// the materialised file under the strictest policy shape (enabled,
/// `workspace_only`, action dir outside the workspace).
#[tokio::test]
async fn threshold_persists_to_blobs_and_file_read_serves_the_key() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let root = crate::security::policy::tool_result_artifacts_dir(&workspace);
    let store = in_memory_tool_result_store(root.clone(), "session/one");
    let secret = "Bearer credential-marker-redact-me-123456";
    let raw = format!("{} {}", "x".repeat(4096), secret);

    let (out, outcome) = apply_per_result_persistence(
        raw.clone(),
        None,
        Some(&store),
        "shell",
        Some("call-1"),
        1024,
    )
    .await;

    let key = "harness/tool_results/session_one/shell/call-1.txt";
    assert!(outcome.persisted);
    assert!(out.contains(&format!("artifact_path: {key}\n")), "{out}");
    assert!(out.contains("[preview]"));
    assert!(!out.contains(secret));
    assert!(
        !root.exists(),
        "nothing is on disk until a read asks for it"
    );
    assert_eq!(std::fs::read_dir(&project).unwrap().count(), 0);

    // The model's `file_read {"path": key}` is recognised and pointed at the
    // materialised copy of the blob.
    let mut args = json!({"path": key});
    let read = artifact_read_target(Some(&store), "file_read", &args).expect("artifact read");
    store.resolve_read(&mut args, &read).await;
    let pointer = args["path"].as_str().unwrap().to_string();
    assert_eq!(
        std::path::Path::new(&pointer),
        root.join("session_one/shell/call-1.txt")
    );

    let cfg = crate::config::AutonomyConfig {
        enabled: true,
        workspace_only: true,
        ..crate::config::AutonomyConfig::default()
    };
    let policy = Arc::new(SecurityPolicy::from_config(&cfg, &workspace, &project));
    let reader = FileReadTool::new(policy);
    let read = reader.execute(args).await.unwrap();
    assert!(!read.is_error, "{}", read.output());
    assert!(read.output().contains("xxxx"));
    assert!(!read.output().contains(secret));
}

/// A `use_skill`-wrapped read is rewritten at the call it wraps.
#[tokio::test]
async fn a_wrapped_read_is_pointed_at_the_materialised_file() {
    let tmp = tempfile::tempdir().unwrap();
    let store = in_memory_tool_result_store(tmp.path().join("tool-results"), "s");
    let (_, outcome) = apply_per_result_persistence(
        "y".repeat(4096),
        None,
        Some(&store),
        "shell",
        Some("c"),
        512,
    )
    .await;
    let key = outcome.artifact_path.unwrap();
    let mut args =
        json!({"skill": "files", "tool": "file_read", "args": {"path": key, "offset": 3}});
    let read = artifact_read_target(Some(&store), "use_skill", &args).unwrap();
    store.resolve_read(&mut args, &read).await;
    assert_ne!(args["args"]["path"], json!(key));
    assert_eq!(args["args"]["offset"], json!(3));
    assert!(std::path::Path::new(args["args"]["path"].as_str().unwrap()).is_file());
}

/// A blob store is the tenant's: an artifact another tenant stored is absent
/// from this tenant's store, so its key is never served or materialised.
#[tokio::test]
async fn artifacts_are_tenant_scoped() {
    use tinystoragedrivers::{MemoryStorage, Scope, StorageBackend};
    let tmp = tempfile::tempdir().unwrap();
    let backend = MemoryStorage::new();
    let scoped = |name: &str| {
        let scope = Scope::new(name).unwrap();
        Arc::clone(backend.for_scope(&scope).unwrap().blobs())
    };
    let store_a = new_tool_result_store(scoped("tenant-a"), tmp.path().join("a"), "s");
    let store_b = new_tool_result_store(scoped("tenant-b"), tmp.path().join("b"), "s");

    let (_, outcome) = apply_per_result_persistence(
        "secret ".repeat(1000),
        None,
        Some(&store_a),
        "shell",
        Some("c"),
        512,
    )
    .await;
    let key = outcome.artifact_path.unwrap();

    assert!(store_a.read(&key).await.unwrap().is_some());
    assert!(store_b.read(&key).await.unwrap().is_none());
    let mut args = json!({"path": key});
    let read = artifact_read_target(Some(&store_b), "file_read", &args).unwrap();
    store_b.resolve_read(&mut args, &read).await;
    assert_eq!(args["path"], json!(key), "left unresolved for tenant b");
    assert!(!tmp.path().join("b").exists());
}

/// The vocabulary passed to the crate is OpenHuman's: `file_read` reads,
/// `use_skill` is the only wrapper followed.
#[test]
fn read_targets_use_openhumans_tool_names() {
    let path = "artifacts/tool-results/s/shell/c.txt";
    assert!(artifact_read_target(None, "file_read", &json!({"path": path})).is_some());
    assert!(artifact_read_target(
        None,
        "use_skill",
        &json!({"skill": "files", "tool": "file_read", "args": {"path": path, "offset": 3}})
    )
    .is_some_and(|read| read.offset == 3));
    assert!(artifact_read_target(None, "glob", &json!({"path": path})).is_none());
}

/// With the session's store, its `tool-results/` keys are recognised, wrapped
/// in `use_skill` or not.
#[test]
fn read_targets_recognise_the_stores_blob_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let store = in_memory_tool_result_store(tmp.path().join("tool-results"), "s");
    let pointer = store.path_for_read_tool("shell", Some("c"));
    assert!(artifact_read_target(Some(&store), "file_read", &json!({"path": pointer})).is_some());
    assert!(artifact_read_target(
        Some(&store),
        "use_skill",
        &json!({"skill": "files", "tool": "file_read", "args": {"path": pointer, "offset": 3}})
    )
    .is_some_and(|read| read.offset == 3));
    assert!(artifact_read_target(
        Some(&store),
        "file_read",
        &json!({"path": "tool-results-backup/s/x.txt"})
    )
    .is_none());
}

/// A continuation names `file_read` and the offset, in the wire format the
/// model has always seen.
#[test]
fn a_page_names_file_read_as_the_continuation() {
    let read = ArtifactRead {
        path: "artifacts/tool-results/s/shell/c.txt".to_string(),
        offset: 0,
    };
    let page = page_artifact_read("y".repeat(5_000), &read, 1_000);
    assert!(
        page.contains(
            "Continue with file_read {\"path\":\"artifacts/tool-results/s/shell/c.txt\",\"offset\":"
        ),
        "{page}"
    );
}
