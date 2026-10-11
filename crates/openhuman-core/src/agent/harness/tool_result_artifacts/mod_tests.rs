use super::*;
use crate::security::SecurityPolicy;
use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::artifacts::tool_results::apply_per_result_persistence;
use tinytools::Tool;

/// End to end through OpenHuman's wiring: the real `sanitize_text` redacts the
/// stored body and the preview, the file lands under the workspace rather than
/// the project the agent is editing, and the real `file_read` opens it at the
/// absolute path the envelope names, under the strictest policy shape
/// (enabled, `workspace_only`, action dir outside the workspace).
#[tokio::test]
async fn threshold_persists_outside_the_project_and_reads_back() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    // `detached` adds its own `tool-results` namespace; give it the artifacts
    // directory so the final path matches the policy-granted root.
    let store = new_tool_result_store(workspace.join("artifacts"), "session/one");
    let raw = format!(
        "{} {}",
        "x".repeat(4096),
        "Bearer credential-marker-redact-me-123456"
    );

    let (out, outcome) = apply_per_result_persistence(
        raw.clone(),
        None,
        Some(&store),
        "shell",
        Some("call-1"),
        1024,
    )
    .await;

    let expected = workspace.join("artifacts/tool-results/session_one/shell/call-1.txt");
    let pointer = expected.to_string_lossy().into_owned();
    assert!(outcome.persisted);
    assert!(
        out.contains(&format!("artifact_path: {pointer}\n")),
        "{out}"
    );
    assert!(out.contains("original_bytes:"));
    assert!(out.contains("[preview]"));
    assert!(!out.contains("Bearer credential-marker-redact-me-123456"));
    assert!(expected.is_file());
    assert_eq!(
        std::fs::read_dir(&project).unwrap().count(),
        0,
        "nothing may be written into the project"
    );

    let cfg = crate::config::AutonomyConfig {
        enabled: true,
        workspace_only: true,
        ..crate::config::AutonomyConfig::default()
    };
    let policy = Arc::new(SecurityPolicy::from_config(&cfg, &workspace, &project));
    let reader = FileReadTool::new(policy);
    let read = reader.execute(json!({"path": pointer})).await.unwrap();
    assert!(!read.is_error, "{}", read.output());
    assert!(read.output().contains("xxxx"));
    assert!(!read
        .output()
        .contains("Bearer credential-marker-redact-me-123456"));
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

/// With the session's store, its absolute pointers page like the relative ones
/// did, wrapped in `use_skill` or not.
#[test]
fn read_targets_recognise_the_detached_stores_absolute_pointers() {
    let tmp = tempfile::tempdir().unwrap();
    let store = new_tool_result_store(tmp.path().join("tool-results"), "s");
    let pointer = store.path_for_read_tool("shell", Some("c"));
    assert!(artifact_read_target(Some(&store), "file_read", &json!({"path": pointer})).is_some());
    assert!(artifact_read_target(
        Some(&store),
        "use_skill",
        &json!({"skill": "files", "tool": "file_read", "args": {"path": pointer, "offset": 3}})
    )
    .is_some_and(|read| read.offset == 3));
    // Without the store there is nothing to recognise an absolute path against.
    assert!(artifact_read_target(None, "file_read", &json!({"path": pointer})).is_none());
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
