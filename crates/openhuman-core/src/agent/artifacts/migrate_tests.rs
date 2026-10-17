use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::agent::artifacts::store::{
    get_artifact, list_artifacts, read_artifact_args, read_artifact_bytes, save_artifact_args,
    save_artifact_meta,
};
use crate::agent::artifacts::types::ArtifactKind;

/// A pre-#5505 record: bytes under `<workspace>/artifacts/<id>/<name>`.
async fn legacy(workspace: &Path, id: &str, name: &str, bytes: &[u8], status: ArtifactStatus) {
    let meta = ArtifactMeta {
        id: id.to_string(),
        kind: ArtifactKind::Document,
        title: name.to_string(),
        path: format!("{id}/{name}"),
        file: None,
        file_root: None,
        size_bytes: bytes.len() as u64,
        status,
        created_at: chrono::Utc::now(),
        error: None,
        thread_id: None,
        tool_call_id: None,
    };
    save_artifact_meta(workspace, &meta).await.unwrap();
    save_artifact_args(workspace, id, &serde_json::json!({ "title": name }))
        .await
        .unwrap();
    let dir = workspace.join("artifacts").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), bytes).unwrap();
}

fn legacy_path(workspace: &Path, id: &str, name: &str) -> PathBuf {
    workspace.join("artifacts").join(id).join(name)
}

/// Visible files in the folder (hidden `.partial`s excluded).
fn visible(files_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(files_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

fn partials(files_dir: &Path) -> usize {
    std::fs::read_dir(files_dir)
        .map(|entries| {
            entries
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".partial")
                })
                .count()
        })
        .unwrap_or(0)
}

#[tokio::test]
async fn moves_a_legacy_file_into_the_files_folder() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");
    legacy(
        &ws,
        "a1",
        "q3-deck.pptx",
        b"deck bytes",
        ArtifactStatus::Ready,
    )
    .await;

    let report = migrate_legacy_artifacts(&ws, &files_dir).await;

    assert_eq!(
        report,
        MigrationReport {
            moved: 1,
            cleaned: 0,
            failed: 0
        }
    );
    let meta = get_artifact(&ws, "a1").await.unwrap();
    assert_eq!(
        meta.file.as_deref(),
        Some(files_dir.join("q3-deck.pptx").to_string_lossy().as_ref())
    );
    assert_eq!(
        meta.file_root.as_deref(),
        Some(files_dir.to_string_lossy().as_ref())
    );
    assert_eq!(meta.path, "q3-deck.pptx");
    assert_eq!(
        read_artifact_bytes(&ws, &files_dir, "a1").await.unwrap(),
        b"deck bytes"
    );
    assert!(
        !legacy_path(&ws, "a1", "q3-deck.pptx").exists(),
        "legacy bytes removed"
    );
    assert!(
        read_artifact_args(&ws, "a1").await.is_ok(),
        "metadata never moves"
    );
}

#[tokio::test]
async fn running_twice_changes_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");
    legacy(&ws, "a1", "notes.docx", b"notes", ArtifactStatus::Ready).await;

    migrate_legacy_artifacts(&ws, &files_dir).await;
    let second = migrate_legacy_artifacts(&ws, &files_dir).await;

    assert_eq!(second, MigrationReport::default());
    assert_eq!(visible(&files_dir), vec!["notes.docx"]);
    assert_eq!(
        read_artifact_bytes(&ws, &files_dir, "a1").await.unwrap(),
        b"notes"
    );
}

#[tokio::test]
async fn leaves_pending_and_failed_records_alone() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");
    legacy(&ws, "p1", "half.pptx", b"", ArtifactStatus::Pending).await;
    legacy(&ws, "f1", "broken.pptx", b"", ArtifactStatus::Failed).await;

    let report = migrate_legacy_artifacts(&ws, &files_dir).await;

    assert_eq!(report, MigrationReport::default());
    assert!(get_artifact(&ws, "p1").await.unwrap().file.is_none());
    assert!(get_artifact(&ws, "f1").await.unwrap().file.is_none());
    assert!(visible(&files_dir).is_empty());
}

#[tokio::test]
async fn a_name_already_in_the_folder_is_never_overwritten() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");
    std::fs::create_dir_all(&files_dir).unwrap();
    std::fs::write(files_dir.join("report.txt"), b"the user's own file").unwrap();
    legacy(
        &ws,
        "a1",
        "report.txt",
        b"agent report",
        ArtifactStatus::Ready,
    )
    .await;

    migrate_legacy_artifacts(&ws, &files_dir).await;

    assert_eq!(
        std::fs::read(files_dir.join("report.txt")).unwrap(),
        b"the user's own file"
    );
    assert_eq!(
        get_artifact(&ws, "a1").await.unwrap().path,
        "report (2).txt"
    );
    assert_eq!(
        read_artifact_bytes(&ws, &files_dir, "a1").await.unwrap(),
        b"agent report"
    );
}

/// Two accounts migrate a same-named file into the one shared folder: both
/// survive under distinct names, and each account still lists only its own.
#[tokio::test]
async fn two_accounts_migrate_into_the_shared_folder_without_clobbering() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    let ws_a = tmp.path().join("users/a/workspace");
    let ws_b = tmp.path().join("users/b/workspace");
    legacy(
        &ws_a,
        "a1",
        "budget.xlsx",
        b"a's budget",
        ArtifactStatus::Ready,
    )
    .await;
    legacy(
        &ws_b,
        "b1",
        "budget.xlsx",
        b"b's budget",
        ArtifactStatus::Ready,
    )
    .await;

    migrate_legacy_artifacts(&ws_a, &files_dir).await;
    migrate_legacy_artifacts(&ws_b, &files_dir).await;

    assert_eq!(visible(&files_dir), vec!["budget (2).xlsx", "budget.xlsx"]);
    assert_eq!(
        read_artifact_bytes(&ws_a, &files_dir, "a1").await.unwrap(),
        b"a's budget"
    );
    assert_eq!(
        read_artifact_bytes(&ws_b, &files_dir, "b1").await.unwrap(),
        b"b's budget"
    );
    let (listed, total) = list_artifacts(&ws_b, 0, 50, None).await.unwrap();
    assert_eq!((total, listed[0].id.as_str()), (1, "b1"));
}

// ── Crash safety: stop after each step, check nothing is lost, rerun ──────

#[tokio::test]
async fn a_crash_after_the_copy_leaves_the_record_intact_and_is_redone() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");
    legacy(&ws, "a1", "deck.pptx", b"deck", ArtifactStatus::Ready).await;

    migrate_with(&ws, &files_dir, Some(CrashAfter::Copy)).await;
    assert!(get_artifact(&ws, "a1").await.unwrap().file.is_none());
    assert_eq!(
        read_artifact_bytes(&ws, &files_dir, "a1").await.unwrap(),
        b"deck"
    );
    assert_eq!(partials(&files_dir), 1);

    let report = migrate_legacy_artifacts(&ws, &files_dir).await;
    assert_eq!(report.moved, 1);
    assert_eq!(partials(&files_dir), 0, "the stale partial is removed");
    assert_eq!(visible(&files_dir), vec!["deck.pptx"]);
    assert_eq!(
        read_artifact_bytes(&ws, &files_dir, "a1").await.unwrap(),
        b"deck"
    );
}

#[tokio::test]
async fn a_crash_after_the_rename_loses_nothing_and_leaves_at_most_one_duplicate() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");
    legacy(&ws, "a1", "deck.pptx", b"deck", ArtifactStatus::Ready).await;

    migrate_with(&ws, &files_dir, Some(CrashAfter::Rename)).await;
    assert!(get_artifact(&ws, "a1").await.unwrap().file.is_none());
    assert_eq!(
        read_artifact_bytes(&ws, &files_dir, "a1").await.unwrap(),
        b"deck"
    );

    migrate_legacy_artifacts(&ws, &files_dir).await;
    let meta = get_artifact(&ws, "a1").await.unwrap();
    assert!(meta.file.is_some());
    assert_eq!(
        read_artifact_bytes(&ws, &files_dir, "a1").await.unwrap(),
        b"deck"
    );
    assert!(visible(&files_dir).len() <= 2);
    assert!(!legacy_path(&ws, "a1", "deck.pptx").exists());
}

#[tokio::test]
async fn a_crash_after_the_meta_write_is_finished_by_the_next_run() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");
    legacy(&ws, "a1", "deck.pptx", b"deck", ArtifactStatus::Ready).await;

    migrate_with(&ws, &files_dir, Some(CrashAfter::MetaWrite)).await;
    assert!(get_artifact(&ws, "a1").await.unwrap().file.is_some());
    assert!(
        legacy_path(&ws, "a1", "deck.pptx").exists(),
        "legacy copy not yet removed"
    );
    assert_eq!(
        read_artifact_bytes(&ws, &files_dir, "a1").await.unwrap(),
        b"deck"
    );

    let report = migrate_legacy_artifacts(&ws, &files_dir).await;
    assert_eq!(
        report,
        MigrationReport {
            moved: 0,
            cleaned: 1,
            failed: 0
        }
    );
    assert!(!legacy_path(&ws, "a1", "deck.pptx").exists());
    assert_eq!(visible(&files_dir), vec!["deck.pptx"]);
    assert!(
        read_artifact_args(&ws, "a1").await.is_ok(),
        "sidecars are never cleaned"
    );
}

/// A partial left by an interrupted run whose record has since gone (deleted
/// between runs) is not overwritten by any later copy; the sweep removes it.
#[tokio::test]
async fn an_orphaned_partial_from_an_earlier_run_is_removed() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");
    std::fs::create_dir_all(&files_dir).unwrap();
    std::fs::write(files_dir.join(".deleted-record.partial"), b"half a deck").unwrap();
    legacy(&ws, "a1", "notes.docx", b"notes", ArtifactStatus::Ready).await;

    migrate_legacy_artifacts(&ws, &files_dir).await;

    assert_eq!(partials(&files_dir), 0);
    assert_eq!(visible(&files_dir), vec!["notes.docx"]);
}
