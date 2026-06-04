use tempfile::TempDir;

use super::*;
use crate::openhuman::config::Config;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().to_path_buf(),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

// ── ai_list_artifacts ──────────────────────────────────────────────────────

#[tokio::test]
async fn list_empty() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let outcome = ai_list_artifacts(&config, None, None, None).await.unwrap();
    let value = outcome.into_cli_compatible_json().unwrap();
    assert_eq!(value["total"], 0);
    assert_eq!(value["artifacts"].as_array().unwrap().len(), 0);
    assert_eq!(value["offset"], 0);
    assert_eq!(value["limit"], DEFAULT_LIMIT as u64);
}

/// #3226: when no `thread_id` filter is supplied the listing surfaces
/// artifacts from every thread (and the legacy ones without a thread).
#[tokio::test]
async fn list_without_thread_filter_returns_all_threads() {
    use crate::openhuman::artifacts::store::save_artifact_meta;
    use crate::openhuman::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    for (id, tid) in [
        ("a1", Some("thread-a".to_string())),
        ("b1", Some("thread-b".to_string())),
        ("legacy", None),
    ] {
        save_artifact_meta(
            tmp.path(),
            &ArtifactMeta {
                id: id.to_string(),
                kind: ArtifactKind::Document,
                title: id.to_string(),
                path: format!("{id}/x.txt"),
                size_bytes: 0,
                status: ArtifactStatus::Ready,
                created_at: chrono::Utc::now(),
                error: None,
                thread_id: tid,
            },
        )
        .await
        .unwrap();
    }

    let outcome = ai_list_artifacts(&config, None, None, None).await.unwrap();
    let value = outcome.into_cli_compatible_json().unwrap();
    assert_eq!(value["total"], 3, "unfiltered list returns every artifact");
}

/// #3226: `thread_id = Some(_)` returns ONLY artifacts whose persisted
/// meta matches that thread. Legacy entries (thread_id absent) are
/// deliberately excluded — they have no addressable owning thread.
#[tokio::test]
async fn list_with_thread_filter_returns_only_matching_thread() {
    use crate::openhuman::artifacts::store::save_artifact_meta;
    use crate::openhuman::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    for (id, tid) in [
        ("a1", Some("thread-a".to_string())),
        ("a2", Some("thread-a".to_string())),
        ("b1", Some("thread-b".to_string())),
        ("legacy", None),
    ] {
        save_artifact_meta(
            tmp.path(),
            &ArtifactMeta {
                id: id.to_string(),
                kind: ArtifactKind::Document,
                title: id.to_string(),
                path: format!("{id}/x.txt"),
                size_bytes: 0,
                status: ArtifactStatus::Ready,
                created_at: chrono::Utc::now(),
                error: None,
                thread_id: tid,
            },
        )
        .await
        .unwrap();
    }

    let outcome = ai_list_artifacts(&config, None, None, Some("thread-a"))
        .await
        .unwrap();
    let value = outcome.into_cli_compatible_json().unwrap();
    assert_eq!(value["total"], 2, "only two artifacts belong to thread-a");
    let ids: Vec<&str> = value["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"a1"));
    assert!(ids.contains(&"a2"));
    assert!(
        !ids.contains(&"b1"),
        "thread-b leaked into thread-a listing"
    );
    assert!(
        !ids.contains(&"legacy"),
        "legacy artifact (thread_id=None) must NOT match a specific thread filter"
    );
}

/// #3226: when the filtered thread has no artifacts, `total` is 0
/// (per-thread, not per-workspace) so the UI's "showing N of M" line
/// stays meaningful.
#[tokio::test]
async fn list_with_thread_filter_unknown_thread_returns_zero() {
    use crate::openhuman::artifacts::store::save_artifact_meta;
    use crate::openhuman::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    save_artifact_meta(
        tmp.path(),
        &ArtifactMeta {
            id: "only".to_string(),
            kind: ArtifactKind::Document,
            title: "only".to_string(),
            path: "only/x.txt".to_string(),
            size_bytes: 0,
            status: ArtifactStatus::Ready,
            created_at: chrono::Utc::now(),
            error: None,
            thread_id: Some("thread-a".to_string()),
        },
    )
    .await
    .unwrap();
    let outcome = ai_list_artifacts(&config, None, None, Some("thread-missing"))
        .await
        .unwrap();
    let value = outcome.into_cli_compatible_json().unwrap();
    assert_eq!(value["total"], 0);
    assert_eq!(value["artifacts"].as_array().unwrap().len(), 0);
}

// ── ai_get_artifact ────────────────────────────────────────────────────────

#[tokio::test]
async fn get_missing_id_error() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = ai_get_artifact(&config, "").await.unwrap_err();
    assert!(err.contains("must not be empty"), "unexpected error: {err}");
}

// ── ai_delete_artifact ─────────────────────────────────────────────────────

#[tokio::test]
async fn delete_missing_id_error() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = ai_delete_artifact(&config, "").await.unwrap_err();
    assert!(err.contains("must not be empty"), "unexpected error: {err}");
}
