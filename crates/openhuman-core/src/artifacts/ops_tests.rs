use tempfile::TempDir;

use super::*;
use crate::config::Config;

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
    use crate::artifacts::store::save_artifact_meta;
    use crate::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
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
                file: None,
                file_root: None,
                size_bytes: 0,
                status: ArtifactStatus::Ready,
                created_at: chrono::Utc::now(),
                error: None,
                thread_id: tid,
                tool_call_id: None,
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
    use crate::artifacts::store::save_artifact_meta;
    use crate::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
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
                file: None,
                file_root: None,
                size_bytes: 0,
                status: ArtifactStatus::Ready,
                created_at: chrono::Utc::now(),
                error: None,
                thread_id: tid,
                tool_call_id: None,
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
    use crate::artifacts::store::save_artifact_meta;
    use crate::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    save_artifact_meta(
        tmp.path(),
        &ArtifactMeta {
            id: "only".to_string(),
            kind: ArtifactKind::Document,
            title: "only".to_string(),
            path: "only/x.txt".to_string(),
            file: None,
            file_root: None,
            size_bytes: 0,
            status: ArtifactStatus::Ready,
            created_at: chrono::Utc::now(),
            error: None,
            thread_id: Some("thread-a".to_string()),
            tool_call_id: None,
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

// ── ai_regenerate (#3162) ──────────────────────────────────────────────────

#[tokio::test]
async fn regenerate_rejects_empty_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(ai_regenerate(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "",
        "t",
        "c"
    )
    .await
    .is_err());
}

#[tokio::test]
async fn regenerate_requires_thread_and_client() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(ai_regenerate(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "id",
        "",
        "c"
    )
    .await
    .is_err());
    assert!(ai_regenerate(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "id",
        "t",
        ""
    )
    .await
    .is_err());
}

#[tokio::test]
async fn regenerate_rejects_non_presentation_kind() {
    use crate::artifacts::store::{save_artifact_args, save_artifact_meta};
    use crate::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    save_artifact_meta(
        tmp.path(),
        &ArtifactMeta {
            id: "doc-1".to_string(),
            kind: ArtifactKind::Document,
            title: "notes".to_string(),
            path: "doc-1/notes.txt".to_string(),
            file: None,
            file_root: None,
            size_bytes: 0,
            status: ArtifactStatus::Failed,
            created_at: chrono::Utc::now(),
            error: Some("boom".to_string()),
            thread_id: Some("t".to_string()),
            tool_call_id: None,
        },
    )
    .await
    .unwrap();
    save_artifact_args(tmp.path(), "doc-1", &serde_json::json!({}))
        .await
        .unwrap();

    let err = ai_regenerate(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "doc-1",
        "t",
        "c",
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("only supported for presentations"),
        "unexpected error: {err}"
    );
}

// Both regeneration tests drive a *presentation* artifact, and the producer
// behind that lives under the `documents` gate — with it off, `ai_regenerate`
// short-circuits with "built without the `documents` feature" before it can
// reach either behaviour these tests are about.
#[cfg(feature = "documents")]
#[tokio::test]
async fn regenerate_errors_when_args_missing() {
    use crate::artifacts::store::create_artifact;
    use crate::artifacts::types::ArtifactKind;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A presentation artifact with no persisted args.json (e.g. created
    // before #3162) cannot be regenerated.
    let (meta, _) = create_artifact(
        tmp.path(),
        &tmp.path().join("Files"),
        ArtifactKind::Presentation,
        "Old Deck",
        "pptx",
        None,
    )
    .await
    .unwrap();
    let err = ai_regenerate(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &meta.id,
        "t",
        "c",
    )
    .await
    .unwrap_err();
    assert!(err.contains("not regenerable"), "unexpected error: {err}");
}

#[cfg(feature = "documents")]
#[tokio::test]
async fn regenerate_reruns_producer_and_reuses_id() {
    if crate::util::module_test_process::run(
        module_path!(),
        "regenerate_reruns_producer_and_reuses_id",
    ) {
        return;
    }
    use crate::artifacts::store::{create_artifact, get_artifact, save_artifact_args};
    use crate::artifacts::types::{ArtifactKind, ArtifactStatus};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Seed a presentation artifact + its persisted creation args.
    let (meta, _) = create_artifact(
        tmp.path(),
        &tmp.path().join("Files"),
        ArtifactKind::Presentation,
        "Q3 Deck",
        "pptx",
        None,
    )
    .await
    .unwrap();
    let args = serde_json::json!({
        "title": "Q3 Deck",
        "slides": [{ "title": "Intro", "bullets": ["alpha", "beta"] }],
    });
    save_artifact_args(tmp.path(), &meta.id, &args)
        .await
        .unwrap();

    let outcome = ai_regenerate(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &meta.id,
        "thread-1",
        "client-1",
    )
    .await
    .unwrap();
    let value = outcome.into_cli_compatible_json().unwrap();
    assert_eq!(value["artifact_id"], meta.id);
    assert_eq!(value["regenerated"], true);
    // Read the producer's persisted failure before asserting the thin RPC
    // result so a failed re-dispatch reports the underlying reason.
    let got = get_artifact(tmp.path(), &meta.id).await.unwrap();
    assert_eq!(
        value["is_error"], false,
        "regeneration unexpectedly returned a tool error: {value}; artifact error: {:?}",
        got.error
    );

    // Same id reused in place; the re-run drove it to Ready.
    assert_eq!(got.id, meta.id);
    assert_eq!(got.status, ArtifactStatus::Ready);
}

// ── #5505: files folder ─────────────────────────────────────────────────────

/// Points the default files folder (`OPENHUMAN_PROJECTS_DIR/Files`) at a temp
/// dir for one test, holding the shared env lock, and restores it on drop.
struct DefaultFilesDir {
    _lock: tokio::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
    files_dir: std::path::PathBuf,
}

impl DefaultFilesDir {
    async fn under(tmp: &TempDir) -> Self {
        let lock = crate::config::TEST_ENV_LOCK.lock().await;
        let previous = std::env::var_os("OPENHUMAN_PROJECTS_DIR");
        let projects = tmp.path().join("projects");
        unsafe {
            std::env::set_var("OPENHUMAN_PROJECTS_DIR", &projects);
        }
        Self {
            _lock: lock,
            previous,
            files_dir: crate::config::default_files_dir(),
        }
    }
}

impl Drop for DefaultFilesDir {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(v) => std::env::set_var("OPENHUMAN_PROJECTS_DIR", v),
                None => std::env::remove_var("OPENHUMAN_PROJECTS_DIR"),
            }
        }
    }
}

#[tokio::test]
async fn get_returns_the_files_folder_path() {
    use crate::artifacts::store::{create_artifact, finalize_artifact};
    use crate::artifacts::types::ArtifactKind;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let default = DefaultFilesDir::under(&tmp).await;
    let files_dir = default.files_dir.clone();
    let (meta, path) = create_artifact(
        tmp.path(),
        &files_dir,
        ArtifactKind::Document,
        "Plan",
        "docx",
        None,
    )
    .await
    .unwrap();
    std::fs::write(&path, b"plan").unwrap();
    finalize_artifact(tmp.path(), &meta.id, 4, None)
        .await
        .unwrap();

    let value = ai_get_artifact(&config, &meta.id)
        .await
        .unwrap()
        .into_cli_compatible_json()
        .unwrap();

    assert_eq!(
        value["absolute_path"],
        files_dir.join("plan.docx").to_string_lossy().as_ref()
    );
}

#[tokio::test]
async fn get_reports_a_ready_file_removed_outside_openhuman() {
    use crate::artifacts::store::{create_artifact, finalize_artifact};
    use crate::artifacts::types::ArtifactKind;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let default = DefaultFilesDir::under(&tmp).await;
    let (meta, path) = create_artifact(
        tmp.path(),
        &default.files_dir,
        ArtifactKind::Document,
        "Plan",
        "docx",
        None,
    )
    .await
    .unwrap();
    std::fs::write(&path, b"plan").unwrap();
    finalize_artifact(tmp.path(), &meta.id, 4, None)
        .await
        .unwrap();
    std::fs::remove_file(&path).unwrap();

    let err = ai_get_artifact(&config, &meta.id).await.unwrap_err();
    assert!(err.contains("file missing"), "{err}");
}

/// A record whose `file_root` is not a files folder the core vouches for is
/// refused by get, and delete leaves the file it names alone.
#[tokio::test]
async fn get_and_delete_refuse_a_record_rooted_outside_the_files_folder() {
    use crate::artifacts::store::save_artifact_meta;
    use crate::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let _default = DefaultFilesDir::under(&tmp).await;
    let home = tmp.path().join("home");
    let private = home.join("private.pdf");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(&private, b"private").unwrap();
    save_artifact_meta(
        tmp.path(),
        &ArtifactMeta {
            id: "claims-home".to_string(),
            kind: ArtifactKind::Document,
            title: "x".to_string(),
            path: "private.pdf".to_string(),
            file: Some(private.to_string_lossy().into_owned()),
            file_root: Some(home.to_string_lossy().into_owned()),
            size_bytes: 7,
            status: ArtifactStatus::Ready,
            created_at: chrono::Utc::now(),
            error: None,
            thread_id: None,
            tool_call_id: None,
        },
    )
    .await
    .unwrap();

    let err = ai_get_artifact(&config, "claims-home").await.unwrap_err();
    assert!(err.contains("not a files folder"), "{err}");

    ai_delete_artifact(&config, "claims-home").await.unwrap();
    assert!(
        private.exists(),
        "the file outside the files folder is left alone"
    );
}

#[tokio::test]
async fn sharing_without_cloud_storage_is_refused_with_a_reason() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = ai_share_artifact(&config, "a-1", None, false)
        .await
        .unwrap_err();
    assert!(err.contains("sign in"), "{err}");
}
