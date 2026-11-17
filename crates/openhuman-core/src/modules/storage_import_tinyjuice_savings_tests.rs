use super::*;
use crate::storage::{Scope, Storage};
use std::sync::atomic::{AtomicUsize, Ordering};
use tinystoragedrivers_testkit::memory_storage;

const SNAPSHOT: &str = r#"{
    "total":{"events":2,"originalTokens":4000,"compactedTokens":2000,"tokensSaved":2000,"costSavedUsd":0.006},
    "byModel":{"m":{"events":2,"originalTokens":4000,"compactedTokens":2000,"tokensSaved":2000,"costSavedUsd":0.006}},
    "byCompressor":{"smartcrusher":{"events":2,"originalTokens":4000,"compactedTokens":2000,"tokensSaved":2000,"costSavedUsd":0.006}}
}"#;

fn scoped() -> Storage {
    memory_storage(&Scope::new("module:tinyjuice@local").unwrap())
}

fn workspace_with(snapshot: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if let Some(text) = snapshot {
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        std::fs::write(legacy_file(dir.path()), text).unwrap();
    }
    dir
}

#[tokio::test]
async fn the_legacy_snapshot_is_handed_to_the_module_once_and_left_in_place() {
    let ws = workspace_with(Some(SNAPSHOT));
    let file = legacy_file(ws.path());
    let scoped = scoped();
    let calls = AtomicUsize::new(0);
    let got = std::sync::Mutex::new(None);
    let outcome = import_into(&scoped, &file, |aggregate| {
        calls.fetch_add(1, Ordering::SeqCst);
        *got.lock().unwrap() = Some(aggregate);
        async { Ok(true) }
    })
    .await
    .unwrap();
    assert_eq!(outcome, Outcome::Imported { adopted: true });
    let aggregate = got.lock().unwrap().take().unwrap();
    assert_eq!(aggregate.total.tokens_saved, 2000);
    assert_eq!(aggregate.by_model["m"].events, 2);
    assert!(file.exists(), "the importer never deletes the legacy file");

    let again = import_into(&scoped, &file, |_| async { Ok(true) })
        .await
        .unwrap();
    assert_eq!(again, Outcome::AlreadyDone);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_missing_snapshot_is_no_source_and_writes_no_marker() {
    let ws = workspace_with(None);
    let file = legacy_file(ws.path());
    let scoped = scoped();
    let outcome = import_into(&scoped, &file, |_| async { Ok(true) })
        .await
        .unwrap();
    assert_eq!(outcome, Outcome::NoSource);
    std::fs::create_dir_all(ws.path().join("state")).unwrap();
    std::fs::write(&file, SNAPSHOT).unwrap();
    let later = import_into(&scoped, &file, |_| async { Ok(true) })
        .await
        .unwrap();
    assert_eq!(later, Outcome::Imported { adopted: true });
}

#[tokio::test]
async fn a_corrupt_snapshot_is_skipped_and_not_retried() {
    let ws = workspace_with(Some("{not json"));
    let file = legacy_file(ws.path());
    let scoped = scoped();
    let outcome = import_into(&scoped, &file, |_| async {
        panic!("a corrupt snapshot must not reach the module");
        #[allow(unreachable_code)]
        Ok(true)
    })
    .await
    .unwrap();
    assert_eq!(outcome, Outcome::Imported { adopted: false });
    let again = import_into(&scoped, &file, |_| async { Ok(true) })
        .await
        .unwrap();
    assert_eq!(again, Outcome::AlreadyDone);
}

#[tokio::test]
async fn a_failed_hand_over_leaves_no_marker_so_the_next_start_retries() {
    let ws = workspace_with(Some(SNAPSHOT));
    let file = legacy_file(ws.path());
    let scoped = scoped();
    let failed = import_into(&scoped, &file, |_| async {
        Err(anyhow::anyhow!("bus down"))
    })
    .await;
    assert!(failed.is_err());
    let retry = import_into(&scoped, &file, |_| async { Ok(false) })
        .await
        .unwrap();
    assert_eq!(retry, Outcome::Imported { adopted: false });
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_snapshot_is_not_followed() {
    let ws = workspace_with(None);
    let secret = ws.path().join("elsewhere.json");
    std::fs::write(&secret, SNAPSHOT).unwrap();
    std::fs::create_dir_all(ws.path().join("state")).unwrap();
    let file = legacy_file(ws.path());
    std::os::unix::fs::symlink(&secret, &file).unwrap();
    let outcome = import_into(&scoped(), &file, |_| async {
        panic!("a symlink must not be read");
        #[allow(unreachable_code)]
        Ok(true)
    })
    .await
    .unwrap();
    assert_eq!(outcome, Outcome::NoSource);
}

#[test]
fn the_legacy_path_is_fixed_under_the_workspace_state_dir() {
    let path = legacy_file(Path::new("/w"));
    assert_eq!(path, Path::new("/w/state/tokenjuice_savings.json"));
}
