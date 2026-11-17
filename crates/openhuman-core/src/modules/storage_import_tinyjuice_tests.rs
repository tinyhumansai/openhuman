use super::*;
use crate::storage::{Scope, Storage};
use std::time::Duration;
use tinystoragedrivers_testkit::memory_storage;

const FRESH: &str = "00112233445566778899aabbccddeeff";
const OLD: &str = "ffeeddccbbaa99887766554433221100";
const UPPER: &str = "ABCDEF0123456789ABCDEF0123456789";

fn scoped() -> Storage {
    memory_storage(&Scope::new("module:tinyjuice@local").unwrap())
}

fn set_age(path: &Path, age: Duration) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
}

/// A CCR directory as the pre-storage tier left it: a fresh original, one older
/// than any sensible TTL, and files that are not CCR tokens.
fn legacy_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(FRESH), "fresh original\nline two").unwrap();
    set_age(&dir.path().join(FRESH), Duration::from_secs(60));
    std::fs::write(dir.path().join(OLD), "stale original").unwrap();
    set_age(&dir.path().join(OLD), Duration::from_secs(7200));
    std::fs::write(dir.path().join(UPPER), "upper-case token").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "not a token").unwrap();
    std::fs::write(
        dir.path().join("deadbeefdeadbeefdeadbeefdeadbeef"),
        [0xff, 0xfe, 0x00],
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("0123456789abcdef0123456789abcdef")).unwrap();
    dir
}

async fn read(scoped: &Storage, token: &str) -> Option<String> {
    scoped
        .blobs()
        .get(&ccr_key(token))
        .await
        .unwrap()
        .map(|blob| String::from_utf8(blob.bytes).unwrap())
}

#[tokio::test]
async fn originals_within_their_ttl_move_into_the_modules_blobs() {
    let legacy = legacy_root();
    let scoped = scoped();
    let outcome = import_into(
        &DispatchCtx::host_default(),
        &scoped,
        legacy.path(),
        Some(Duration::from_secs(3600)),
        SystemTime::now(),
    )
    .await
    .unwrap();
    // fresh and the upper-case token are written; the old one is expired; the
    // note, the binary file and the directory are skipped.
    assert_eq!(
        outcome,
        Outcome::Imported {
            imported: 2,
            expired: 1,
            skipped: 3
        }
    );
    assert_eq!(
        read(&scoped, FRESH).await.as_deref(),
        Some("fresh original\nline two")
    );
    assert!(read(&scoped, OLD).await.is_none());
    // The module mints lowercase tokens, so that is the key it will look up.
    assert_eq!(
        read(&scoped, &UPPER.to_ascii_lowercase()).await.as_deref(),
        Some("upper-case token")
    );
}

#[tokio::test]
async fn without_a_ttl_every_readable_original_is_kept() {
    let legacy = legacy_root();
    let scoped = scoped();
    let outcome = import_into(
        &DispatchCtx::host_default(),
        &scoped,
        legacy.path(),
        None,
        SystemTime::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        Outcome::Imported {
            imported: 3,
            expired: 0,
            skipped: 3
        }
    );
    assert_eq!(read(&scoped, OLD).await.as_deref(), Some("stale original"));
}

#[tokio::test]
async fn the_legacy_files_are_left_in_place() {
    let legacy = legacy_root();
    import_into(
        &DispatchCtx::host_default(),
        &scoped(),
        legacy.path(),
        None,
        SystemTime::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(legacy.path().join(FRESH)).unwrap(),
        "fresh original\nline two"
    );
    assert!(legacy.path().join(OLD).exists());
}

#[tokio::test]
async fn a_second_run_is_a_noop_and_a_blob_already_stored_is_kept() {
    let legacy = legacy_root();
    let scoped = scoped();
    scoped
        .blobs()
        .put(&ccr_key(FRESH), b"written by the module".to_vec(), None)
        .await
        .unwrap();
    let first = import_into(
        &DispatchCtx::host_default(),
        &scoped,
        legacy.path(),
        None,
        SystemTime::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        first,
        Outcome::Imported {
            imported: 2,
            expired: 0,
            skipped: 4
        }
    );
    assert_eq!(
        read(&scoped, FRESH).await.as_deref(),
        Some("written by the module")
    );
    assert_eq!(
        import_into(
            &DispatchCtx::host_default(),
            &scoped,
            legacy.path(),
            None,
            SystemTime::now()
        )
        .await
        .unwrap(),
        Outcome::AlreadyDone
    );
}

#[tokio::test]
async fn a_missing_directory_writes_no_marker() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("ccr");
    let scoped = scoped();
    assert_eq!(
        import_into(
            &DispatchCtx::host_default(),
            &scoped,
            &missing,
            None,
            SystemTime::now()
        )
        .await
        .unwrap(),
        Outcome::NoSource
    );
    std::fs::create_dir(&missing).unwrap();
    std::fs::write(missing.join(FRESH), "late").unwrap();
    assert!(matches!(
        import_into(
            &DispatchCtx::host_default(),
            &scoped,
            &missing,
            None,
            SystemTime::now()
        )
        .await
        .unwrap(),
        Outcome::Imported { imported: 1, .. }
    ));
}

#[test]
fn only_hex_tokens_of_the_hash_length_are_ccr_names() {
    assert!(is_token(FRESH));
    for name in [
        "",
        "abc",
        "zz112233445566778899aabbccddeeff",
        "../etc/passwd",
    ] {
        assert!(!is_token(name), "{name}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_token_is_never_followed_into_storage() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "credential bytes").unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join(FRESH)).unwrap();
    let scoped = scoped();
    let outcome = import_into(
        &DispatchCtx::host_default(),
        &scoped,
        dir.path(),
        None,
        SystemTime::now(),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        Outcome::Imported {
            imported: 0,
            skipped: 1,
            ..
        }
    ));
    assert_eq!(read(&scoped, FRESH).await, None);
}
