use std::path::PathBuf;

use tiny_artifacts::{ArtifactKind, ArtifactStatus as RecordStatus, ArtifactStore};

use super::*;
use crate::agent::artifacts::store::get_artifact;
use crate::agent::artifacts::tiny_store::versioned_store;
use crate::agent::artifacts::types::{ArtifactKind as CardKind, ArtifactStatus as CardStatus};

fn temp_dirs() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("production-{}", uuid::Uuid::new_v4()));
    let (workspace, files) = (root.join("workspace"), root.join("Files"));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&files).unwrap();
    (workspace, files)
}

fn files_in(dir: &PathBuf) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

async fn produce(
    workspace: &std::path::Path,
    files: &std::path::Path,
    bytes: &[u8],
    tool_call_id: Option<&str>,
    revise: Option<&str>,
) -> String {
    let production = Production::start(
        workspace,
        FileRoots::new(files),
        ArtifactKind::File,
        "Quarterly Deck",
        "pptx",
        tool_call_id,
        revise,
        None,
    )
    .await
    .unwrap();
    tokio::fs::write(production.path(), bytes).await.unwrap();
    assert_eq!(production.complete().await.unwrap(), bytes.len() as u64);
    production.id().to_string()
}

#[tokio::test]
async fn a_new_artifact_shares_one_id_and_one_file_between_card_and_record() {
    let (workspace, files) = temp_dirs();
    let production = Production::start(
        &workspace,
        FileRoots::new(&files),
        ArtifactKind::File,
        "Quarterly Deck",
        "pptx",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let id = production.id().to_string();

    // While generating: a pending card, and a pending record with no version.
    let card = get_artifact(&workspace, &id).await.unwrap();
    assert_eq!(card.status, CardStatus::Pending);
    assert_eq!(card.kind, CardKind::Presentation);
    let record = versioned_store(&workspace).get(&id).await.unwrap().unwrap();
    assert_eq!(record.status, RecordStatus::Pending);
    assert!(record.versions.is_empty());

    tokio::fs::write(production.path(), b"deck-v1")
        .await
        .unwrap();
    production.complete().await.unwrap();

    let card = get_artifact(&workspace, &id).await.unwrap();
    assert_eq!(card.status, CardStatus::Ready);
    assert_eq!(card.size_bytes, 7);
    let record = versioned_store(&workspace).get(&id).await.unwrap().unwrap();
    assert_eq!(record.status, RecordStatus::Ready);
    assert_eq!(
        record.latest().unwrap().author_id,
        crate::agent::artifacts::tiny_store::AGENT_AUTHOR_ID
    );
    // The version holds its own copy; the files folder shows the newest.
    let file = record.latest().unwrap().file.clone().unwrap();
    let kept = crate::agent::artifacts::tiny_store::version_files_dir(&workspace)
        .join(&id)
        .join("v1.pptx");
    assert_eq!(std::path::Path::new(&file.path), kept.as_path());
    assert_eq!(
        files_in(&files),
        vec!["quarterly-deck.pptx"],
        "no second copy"
    );
}

#[tokio::test]
async fn a_revision_reuses_the_id_and_file_and_keeps_the_history() {
    let (workspace, files) = temp_dirs();
    let id = produce(&workspace, &files, b"deck-v1", Some("call-1"), None).await;
    let again = produce(&workspace, &files, b"deck-v2!", None, Some(&id)).await;
    assert_eq!(again, id);

    let record = versioned_store(&workspace).get(&id).await.unwrap().unwrap();
    assert_eq!(record.versions.len(), 2);
    let card = get_artifact(&workspace, &id).await.unwrap();
    assert_eq!(card.status, CardStatus::Ready);
    assert_eq!(card.size_bytes, 8);
    // The revision keeps the card tied to the call that made the artifact.
    assert_eq!(card.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(files_in(&files), vec!["quarterly-deck.pptx"]);
    assert_eq!(
        std::fs::read(files.join("quarterly-deck.pptx")).unwrap(),
        b"deck-v2!"
    );
    // Each version still holds what it was.
    let bytes_of = |n: u32| std::fs::read(&record.version(n).unwrap().file.clone().unwrap().path);
    assert_eq!(bytes_of(1).unwrap(), b"deck-v1");
    assert_eq!(bytes_of(2).unwrap(), b"deck-v2!");
}

#[tokio::test]
async fn a_failed_production_fails_card_and_record() {
    let (workspace, files) = temp_dirs();
    let production = Production::start(
        &workspace,
        FileRoots::new(&files),
        ArtifactKind::Image,
        "Logo",
        "png",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    production.fail("provider timed out").await;

    let card = get_artifact(&workspace, production.id()).await.unwrap();
    assert_eq!(card.status, CardStatus::Failed);
    assert_eq!(card.error.as_deref(), Some("provider timed out"));
    let record = versioned_store(&workspace)
        .get(production.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, RecordStatus::Failed);
    assert_eq!(record.error.as_deref(), Some("provider timed out"));
}

#[tokio::test]
async fn a_failed_revision_keeps_the_artifact_and_shows_its_newest_version() {
    let (workspace, files) = temp_dirs();
    let id = produce(&workspace, &files, b"deck-v1", None, None).await;

    let revision = Production::start(
        &workspace,
        FileRoots::new(&files),
        ArtifactKind::File,
        "Quarterly Deck",
        "pptx",
        None,
        Some(&id),
        None,
    )
    .await
    .unwrap();
    // The producer got as far as clobbering the file before failing.
    tokio::fs::write(revision.path(), b"half-writ")
        .await
        .unwrap();
    revision.fail("engine crashed").await;

    let record = versioned_store(&workspace).get(&id).await.unwrap().unwrap();
    assert_eq!(record.status, RecordStatus::Ready);
    assert_eq!(record.versions.len(), 1);
    let card = get_artifact(&workspace, &id).await.unwrap();
    assert_eq!(card.status, CardStatus::Ready);
    assert_eq!(
        std::fs::read(files.join("quarterly-deck.pptx")).unwrap(),
        b"deck-v1"
    );
}

#[tokio::test]
async fn a_completion_that_records_nothing_fails_the_record_too() {
    let (workspace, files) = temp_dirs();
    let production = Production::start(
        &workspace,
        FileRoots::new(&files),
        ArtifactKind::File,
        "Quarterly Deck",
        "pptx",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    // No file: the version cannot be recorded.
    let _ = std::fs::remove_file(production.path());
    assert!(production.complete().await.is_err());

    let record = versioned_store(&workspace)
        .get(production.id())
        .await
        .unwrap()
        .unwrap();
    // Not left pending, so its card never stays "generating".
    assert_eq!(record.status, RecordStatus::Failed);
    let card = get_artifact(&workspace, production.id()).await.unwrap();
    assert_eq!(card.status, CardStatus::Failed);
}

#[cfg(unix)]
#[tokio::test]
async fn a_saved_version_the_card_cannot_show_succeeds_and_says_so() {
    use std::os::unix::fs::PermissionsExt;

    // The CI container runs as root, which bypasses the file mode bits used
    // below to make the visible copy unwritable. In that environment the
    // copy succeeds and the card should remain ready; non-root runs exercise
    // the saved-but-not-shown behavior.
    let runs_as_root = unsafe { libc::geteuid() == 0 };

    let (workspace, files) = temp_dirs();
    let production = Production::start(
        &workspace,
        FileRoots::new(&files),
        ArtifactKind::File,
        "Quarterly Deck",
        "pptx",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    tokio::fs::write(production.path(), b"deck-v1")
        .await
        .unwrap();
    // The card's file cannot be written: the version is recorded (from its
    // kept copy), but the card cannot show it.
    let visible = production.path().to_path_buf();
    std::fs::set_permissions(&visible, std::fs::Permissions::from_mode(0o444)).unwrap();

    let result = production.complete().await;
    std::fs::set_permissions(&visible, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(result.unwrap(), 7);

    let record = versioned_store(&workspace)
        .get(production.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, RecordStatus::Ready);
    assert_eq!(record.versions.len(), 1);
    let card = get_artifact(&workspace, production.id()).await.unwrap();
    if runs_as_root {
        assert_eq!(card.status, CardStatus::Ready);
        return;
    }
    assert_eq!(card.status, CardStatus::Failed);
    let error = card.error.unwrap_or_default();
    assert!(error.starts_with(SAVED_NOT_SHOWN), "{error}");
}
