use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::agent::artifacts::store::{
    create_artifact, create_artifact_regenerating, delete_artifact, fail_artifact,
    finalize_artifact, list_artifacts, read_artifact_bytes, save_artifact_meta,
};
use crate::agent::artifacts::types::ArtifactKind;

async fn create_ready(
    workspace: &Path,
    files_dir: &Path,
    title: &str,
    bytes: &[u8],
) -> ArtifactMeta {
    let (meta, path) = create_artifact(
        workspace,
        files_dir,
        ArtifactKind::Document,
        title,
        "txt",
        None,
    )
    .await
    .expect("create_artifact");
    tokio::fs::write(&path, bytes).await.unwrap();
    finalize_artifact(workspace, &meta.id, bytes.len() as u64, None)
        .await
        .expect("finalize_artifact")
}

fn legacy_meta(id: &str, path: &str) -> ArtifactMeta {
    ArtifactMeta {
        id: id.to_string(),
        kind: ArtifactKind::Document,
        title: "Legacy".to_string(),
        path: path.to_string(),
        file: None,
        file_root: None,
        size_bytes: 3,
        status: ArtifactStatus::Ready,
        created_at: chrono::Utc::now(),
        error: None,
        thread_id: None,
        tool_call_id: None,
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn new_artifact_is_written_to_the_files_folder_not_the_workspace() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    let files_dir = tmp.path().join("Files");

    let (meta, path) = create_artifact(
        &workspace,
        &files_dir,
        ArtifactKind::Presentation,
        "Q3 Deck",
        "pptx",
        None,
    )
    .await
    .unwrap();

    assert_eq!(path, files_dir.join("q3-deck.pptx"));
    assert!(path.is_file(), "the name is claimed with a placeholder");
    assert_eq!(meta.path, "q3-deck.pptx");
    assert_eq!(meta.file.as_deref(), Some(path.to_string_lossy().as_ref()));
    assert_eq!(
        meta.file_root.as_deref(),
        Some(files_dir.to_string_lossy().as_ref())
    );
    // The record is a document, not a file: no bytes land in the workspace.
    assert!(!workspace.join("artifacts").join(&meta.id).exists());
}

#[tokio::test]
async fn files_folder_is_created_on_first_use() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("not").join("yet").join("Files");
    create_artifact(
        tmp.path(),
        &files_dir,
        ArtifactKind::Document,
        "Notes",
        "docx",
        None,
    )
    .await
    .unwrap();
    assert!(files_dir.is_dir());
}

#[tokio::test]
async fn a_repeated_title_gets_a_numbered_name_instead_of_overwriting() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    let first = create_ready(tmp.path(), &files_dir, "Report", b"one").await;
    let second = create_ready(tmp.path(), &files_dir, "Report", b"two").await;

    assert_eq!(first.path, "report.txt");
    assert_eq!(second.path, "report (2).txt");
    assert_eq!(
        read_artifact_bytes(tmp.path(), &files_dir, &first.id)
            .await
            .unwrap(),
        b"one"
    );
    assert_eq!(
        read_artifact_bytes(tmp.path(), &files_dir, &second.id)
            .await
            .unwrap(),
        b"two"
    );
}

/// The files folder is shared by every account on the OS user; metadata is
/// not. Two accounts saving the same title must get distinct files, and each
/// account's listing must show only its own records.
#[tokio::test]
async fn two_accounts_share_the_files_folder_without_clobbering_or_leaking() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    let ws_a = tmp.path().join("users").join("a").join("workspace");
    let ws_b = tmp.path().join("users").join("b").join("workspace");

    let a = create_ready(&ws_a, &files_dir, "Budget", b"account a").await;
    let b = create_ready(&ws_b, &files_dir, "Budget", b"account b").await;

    assert_ne!(a.file, b.file);
    assert_eq!(
        read_artifact_bytes(&ws_a, &files_dir, &a.id).await.unwrap(),
        b"account a"
    );
    assert_eq!(
        read_artifact_bytes(&ws_b, &files_dir, &b.id).await.unwrap(),
        b"account b"
    );

    let (listed_b, total_b) = list_artifacts(&ws_b, 0, 50, None).await.unwrap();
    assert_eq!(total_b, 1);
    assert_eq!(listed_b[0].id, b.id);
    assert!(
        get_artifact(&ws_b, &a.id).await.is_err(),
        "B cannot open A's record"
    );
}

#[tokio::test]
async fn regenerate_overwrites_the_file_it_owns() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    let original = create_ready(tmp.path(), &files_dir, "Deck", b"v1").await;

    let (again, path) = create_artifact_regenerating(
        tmp.path(),
        &files_dir,
        ArtifactKind::Document,
        "Deck",
        "txt",
        None,
        Some(original.id.as_str()),
        None,
    )
    .await
    .unwrap();

    assert_eq!(again.id, original.id);
    assert_eq!(again.file, original.file);
    assert_eq!(path, PathBuf::from(original.file.clone().unwrap()));
    assert_eq!(names_in(&files_dir), vec!["deck.txt"]);
}

#[tokio::test]
async fn a_failed_generation_leaves_no_empty_file_behind() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    let (meta, path) = create_artifact(
        tmp.path(),
        &files_dir,
        ArtifactKind::Document,
        "Doomed",
        "docx",
        None,
    )
    .await
    .unwrap();
    assert!(path.is_file());

    fail_artifact(tmp.path(), &files_dir, &meta.id, "engine exploded", None)
        .await
        .unwrap();

    assert!(!path.exists(), "the zero-byte placeholder is removed");
}

#[tokio::test]
async fn delete_removes_the_file_and_the_record() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    let meta = create_ready(tmp.path(), &files_dir, "Gone", b"bye").await;
    let file = PathBuf::from(meta.file.clone().unwrap());

    delete_artifact(tmp.path(), &files_dir, &meta.id)
        .await
        .unwrap();

    assert!(!file.exists());
    assert!(!tmp.path().join("artifacts").join(&meta.id).exists());
}

#[tokio::test]
async fn a_file_removed_outside_openhuman_is_reported_missing() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    let meta = create_ready(tmp.path(), &files_dir, "Moved", b"abc").await;
    std::fs::remove_file(meta.file.as_deref().unwrap()).unwrap();

    let err = read_artifact_bytes(tmp.path(), &files_dir, &meta.id)
        .await
        .unwrap_err();
    assert!(err.contains("file missing"), "{err}");
    let err = resolve_ready_file(tmp.path(), &FileRoots::new(&files_dir), &meta.id)
        .await
        .unwrap_err();
    assert!(err.contains("file missing"), "{err}");
}

async fn tampered(workspace: &Path, file: &Path, file_root: &Path) -> String {
    let mut meta = legacy_meta("tampered", "x.txt");
    meta.file = Some(file.to_string_lossy().into_owned());
    meta.file_root = Some(file_root.to_string_lossy().into_owned());
    save_artifact_meta(workspace, &meta).await.unwrap();
    // Trust the record's own root here so these cases exercise the checks
    // *within* a root; `a_record_pointing_outside_the_files_folders_is_refused`
    // covers a root the core does not vouch for.
    read_artifact_bytes(workspace, file_root, "tampered")
        .await
        .unwrap_err()
}

#[tokio::test]
async fn the_escape_guard_rejects_a_file_outside_its_folder() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    std::fs::create_dir_all(&files_dir).unwrap();
    let secret = tmp.path().join("secret.txt");
    std::fs::write(&secret, b"private").unwrap();

    let err = tampered(tmp.path(), &secret, &files_dir).await;
    assert!(err.contains("escape guard"), "{err}");

    let dotdot = files_dir.join("..").join("secret.txt");
    let err = tampered(tmp.path(), &dotdot, &files_dir).await;
    assert!(err.contains("escape guard"), "{err}");
}

#[tokio::test]
async fn the_escape_guard_keeps_the_credential_store_floor() {
    let tmp = TempDir::new().unwrap();
    let ssh = tmp.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    let key = ssh.join("id_ed25519");
    std::fs::write(&key, b"-----BEGIN").unwrap();

    // Even a record whose root claims the key's parent is refused.
    let err = tampered(tmp.path(), &key, tmp.path()).await;
    assert!(err.contains("escape guard"), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn the_escape_guard_rejects_a_symlink_out_of_the_folder() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    std::fs::create_dir_all(&files_dir).unwrap();
    let secret = tmp.path().join("secret.txt");
    std::fs::write(&secret, b"private").unwrap();
    let link = files_dir.join("innocent.txt");
    std::os::unix::fs::symlink(&secret, &link).unwrap();

    let err = tampered(tmp.path(), &link, &files_dir).await;
    assert!(err.contains("escape guard"), "{err}");
}

#[tokio::test]
async fn a_legacy_record_still_resolves_under_the_workspace() {
    let tmp = TempDir::new().unwrap();
    let meta = legacy_meta("legacy-1", "legacy-1/old.txt");
    save_artifact_meta(tmp.path(), &meta).await.unwrap();
    std::fs::create_dir_all(tmp.path().join("artifacts/legacy-1")).unwrap();
    std::fs::write(tmp.path().join("artifacts/legacy-1/old.txt"), b"old").unwrap();

    assert_eq!(
        read_artifact_bytes(tmp.path(), tmp.path(), "legacy-1")
            .await
            .unwrap(),
        b"old"
    );
}

/// `root.join("../x").starts_with(root)` is true lexically, so the legacy
/// guard must reject `..` components rather than rely on `starts_with`.
#[tokio::test]
async fn a_legacy_path_with_parent_components_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("ws");
    std::fs::write(tmp.path().join("secret.txt"), b"private").unwrap();
    let meta = legacy_meta("legacy-2", "../../secret.txt");
    save_artifact_meta(&workspace, &meta).await.unwrap();

    let err = read_artifact_bytes(&workspace, tmp.path(), "legacy-2")
        .await
        .unwrap_err();
    assert!(err.contains("escapes artifacts root"), "{err}");
}

/// A dangling symlink must be refused by the guard, not skipped: if its target
/// appears later, an unchecked read would follow it out of the folder.
#[cfg(unix)]
#[tokio::test]
async fn the_escape_guard_rejects_a_dangling_symlink() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    std::fs::create_dir_all(&files_dir).unwrap();
    let link = files_dir.join("dangling.txt");
    std::os::unix::fs::symlink(tmp.path().join("not-yet-there.txt"), &link).unwrap();

    let err = tampered(tmp.path(), &link, &files_dir).await;
    assert!(err.contains("could not be resolved"), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn the_escape_guard_rejects_a_symlink_loop() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    std::fs::create_dir_all(&files_dir).unwrap();
    let a = files_dir.join("a.txt");
    let b = files_dir.join("b.txt");
    std::os::unix::fs::symlink(&b, &a).unwrap();
    std::os::unix::fs::symlink(&a, &b).unwrap();

    let err = tampered(tmp.path(), &a, &files_dir).await;
    assert!(err.contains("could not be resolved"), "{err}");
}

/// A files folder that cannot be resolved (here, itself a symlink loop) must
/// not let the check through either.
#[cfg(unix)]
#[tokio::test]
async fn the_escape_guard_rejects_an_unresolvable_folder() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    let other = tmp.path().join("Other");
    std::os::unix::fs::symlink(&other, &files_dir).unwrap();
    std::os::unix::fs::symlink(&files_dir, &other).unwrap();

    let err = tampered(tmp.path(), &files_dir.join("doc.txt"), &files_dir).await;
    assert!(err.contains("could not be resolved"), "{err}");
}

/// A path with nothing at it passes the guard and is reported as missing.
#[tokio::test]
async fn a_recorded_file_that_does_not_exist_reads_as_missing() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    std::fs::create_dir_all(&files_dir).unwrap();

    let err = tampered(tmp.path(), &files_dir.join("gone.txt"), &files_dir).await;
    assert!(err.contains("file missing"), "{err}");
}

/// The record's `file_root` is not trusted on its own say-so: a record that
/// claims a home-directory root and points at a private file inside it passes
/// every within-root check, and is still refused because that root is not one
/// of the folders the core vouches for.
#[tokio::test]
async fn a_record_pointing_outside_the_files_folders_is_refused() {
    let tmp = TempDir::new().unwrap();
    let files_dir = tmp.path().join("Files");
    std::fs::create_dir_all(&files_dir).unwrap();
    let home = tmp.path().join("home");
    let private = home.join("Documents").join("private.pdf");
    std::fs::create_dir_all(private.parent().unwrap()).unwrap();
    std::fs::write(&private, b"private").unwrap();

    let mut meta = legacy_meta("claims-home", "private.pdf");
    meta.file = Some(private.to_string_lossy().into_owned());
    meta.file_root = Some(home.to_string_lossy().into_owned());
    save_artifact_meta(tmp.path(), &meta).await.unwrap();

    let err = read_artifact_bytes(tmp.path(), &files_dir, "claims-home")
        .await
        .unwrap_err();
    assert!(err.contains("not a files folder"), "{err}");
    let err = resolve_ready_file(tmp.path(), &FileRoots::new(&files_dir), "claims-home")
        .await
        .unwrap_err();
    assert!(err.contains("not a files folder"), "{err}");
}

/// A folder the user moved away from stays trusted for the records made there.
#[tokio::test]
async fn a_previously_trusted_folder_still_resolves() {
    let tmp = TempDir::new().unwrap();
    let old = tmp.path().join("Old");
    let meta = create_ready(tmp.path(), &old, "Kept", b"kept").await;

    let roots = FileRoots::new(tmp.path().join("New")).with_trusted([old.clone()]);
    assert_eq!(
        read_artifact_bytes(tmp.path(), &roots, &meta.id)
            .await
            .unwrap(),
        b"kept"
    );
    // Without the old folder in the trusted set, the same record is refused.
    let err = read_artifact_bytes(tmp.path(), tmp.path().join("New"), &meta.id)
        .await
        .unwrap_err();
    assert!(err.contains("not a files folder"), "{err}");
}
