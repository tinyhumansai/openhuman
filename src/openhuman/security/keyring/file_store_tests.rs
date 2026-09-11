//! Tests for the shared secrets-file primitives.

use std::collections::VecDeque;
use std::path::Path;

use super::{
    lock_for_write, lock_path_for, quarantine_corrupt, reserve_temp_file, temp_path_for,
    write_atomic,
};

#[test]
fn write_atomic_replaces_contents() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("secrets.json");

    write_atomic(&path, b"first").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"first");

    write_atomic(&path, b"second").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"second");
}

#[test]
fn write_atomic_creates_missing_parents() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir
        .path()
        .join("nested")
        .join("deeper")
        .join("secrets.json");

    write_atomic(&path, b"value").unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"value");
}

/// The regression the unique temp name exists for: a second writer must not be
/// able to collide with the first writer's staging file.
#[test]
fn temp_paths_are_unique_per_call() {
    let path = Path::new("/tmp/openhuman-test/dev-keychain.json");
    let first = temp_path_for(path);
    let second = temp_path_for(path);

    assert_ne!(first, second);
    assert!(first.to_string_lossy().ends_with(".tmp"));
    assert_eq!(first.parent(), path.parent());
}

#[test]
fn write_atomic_leaves_no_temp_files_behind() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("secrets.json");

    write_atomic(&path, b"value").unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}

#[test]
fn reserve_temp_file_skips_a_stale_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let stale = dir.path().join("stale.tmp");
    let fresh = dir.path().join("fresh.tmp");

    std::fs::write(&stale, b"stale").unwrap();
    let mut paths = VecDeque::from([stale.clone(), fresh.clone()]);
    let (claimed, file) = reserve_temp_file(|| paths.pop_front().unwrap()).unwrap();
    drop(file);

    assert_eq!(claimed, fresh);
    assert_eq!(std::fs::read(&stale).unwrap(), b"stale");
}

#[cfg(unix)]
#[test]
fn write_atomic_writes_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("secrets.json");

    write_atomic(&path, b"value").unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "unexpected mode {mode:o}");
}

#[test]
fn lock_path_is_a_sibling_of_the_secrets_file() {
    let path = Path::new("/tmp/openhuman-test/dev-keychain.json");
    assert_eq!(
        lock_path_for(path),
        Path::new("/tmp/openhuman-test/dev-keychain.json.lock")
    );
}

/// The lock must serialize independent holders, not merely independent
/// processes: a second acquirer waits until the first guard is dropped.
#[test]
fn lock_serializes_concurrent_holders() {
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("dev-keychain.json");

    let guard = lock_for_write(&path).unwrap();

    let (tx, rx) = mpsc::channel();
    let contender_path = path.clone();
    let contender = std::thread::spawn(move || {
        let _guard = lock_for_write(&contender_path).unwrap();
        tx.send(()).unwrap();
    });

    // Still held here, so the contender must not have gotten through.
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "the lock let a second holder in while the first was live"
    );

    drop(guard);
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the contender should acquire the lock once it is released");
    contender.join().unwrap();
}

#[test]
fn lock_survives_the_file_being_replaced() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("dev-keychain.json");
    write_atomic(&path, b"before").unwrap();

    let guard = lock_for_write(&path).unwrap();
    // The rename inside `write_atomic` swaps the secrets file's inode; the lock
    // is on the sidecar, so it is unaffected.
    write_atomic(&path, b"after").unwrap();
    drop(guard);

    assert_eq!(std::fs::read(&path).unwrap(), b"after");
    assert!(lock_path_for(&path).exists());
}

#[test]
fn quarantine_moves_the_file_aside_and_reports_the_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("dev-keychain.json");
    std::fs::write(&path, b"{ not json").unwrap();

    let moved = quarantine_corrupt(&path, "json").expect("the file should be moved aside");

    assert!(
        !path.exists(),
        "the corrupt file should no longer be in place"
    );
    assert_eq!(std::fs::read(&moved).unwrap(), b"{ not json");
    assert!(moved.to_string_lossy().contains(".corrupt."));
}

#[test]
fn quarantine_reports_none_when_there_is_nothing_to_move() {
    let dir = tempfile::TempDir::new().unwrap();
    assert!(quarantine_corrupt(&dir.path().join("absent.json"), "json").is_none());
}
