//! Tests for the shared secrets-file primitives.

use std::collections::VecDeque;
use std::path::Path;

use super::{
    lock_for_write, lock_path_for, quarantine_corrupt, reserve_temp_file, temp_path_for,
    write_atomic,
};

#[cfg(unix)]
#[test]
fn durable_replace_syncs_parent_after_rename_and_propagates_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let source = dir.path().join("staged");
    let destination = dir.path().join("secrets.enc");
    std::fs::write(&source, b"encrypted").unwrap();

    let error = super::replace_durably_with_sync(&source, &destination, |parent| {
        assert_eq!(parent, dir.path());
        assert_eq!(std::fs::read(&destination).unwrap(), b"encrypted");
        Err(std::io::Error::other("directory sync failed"))
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "directory sync failed");
    assert!(!source.exists());
}

#[cfg(windows)]
#[test]
fn existing_ciphertext_is_republished_and_verified_before_plaintext_cleanup() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("secrets.enc");
    std::fs::write(&path, b"ciphertext").unwrap();

    let destination_lock = lock_for_write(&path).unwrap();
    super::sync_parent_dir_with_republish(&path, &destination_lock, |destination, bytes| {
        assert_eq!(destination, path);
        assert_eq!(bytes, b"ciphertext");
        super::write_atomic(destination, bytes)
    })
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"ciphertext");
}

#[test]
fn relative_keyring_paths_sync_the_current_directory() {
    assert_eq!(super::parent_dir(Path::new("keyring")), Path::new("."));
}

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

#[test]
fn two_quarantines_in_one_second_keep_both_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("secrets.enc");
    let mut moved = Vec::new();
    for body in [
        b"first".as_slice(),
        b"second".as_slice(),
        b"third".as_slice(),
    ] {
        std::fs::write(&path, body).unwrap();
        moved.push(quarantine_corrupt(&path, "enc").expect("moved aside"));
    }
    let mut names: Vec<_> = moved.iter().collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 3, "no quarantine reused a name: {moved:?}");
    let mut bodies: Vec<_> = moved.iter().map(|p| std::fs::read(p).unwrap()).collect();
    bodies.sort();
    assert_eq!(
        bodies,
        [b"first".to_vec(), b"second".to_vec(), b"third".to_vec()]
    );
}

#[test]
fn quarantine_never_replaces_a_name_another_process_reserved() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("secrets.enc");
    std::fs::write(&path, b"mine").unwrap();
    // Another process already holds every name this second could pick first.
    // Reserve this second and the next, so a clock tick cannot dodge the test.
    let stamp = chrono::Utc::now().timestamp();
    let taken: Vec<_> = [stamp, stamp + 1]
        .iter()
        .map(|s| path.with_extension(format!("enc.corrupt.{s}")))
        .collect();
    for t in &taken {
        std::fs::write(t, b"theirs").unwrap();
    }
    let moved = quarantine_corrupt(&path, "enc").expect("moved aside");
    assert!(!taken.contains(&moved), "{moved:?} was already taken");
    assert!(!path.exists(), "the corrupt original is gone");
    assert_eq!(std::fs::read(&moved).unwrap(), b"mine");
    for t in &taken {
        assert_eq!(std::fs::read(t).unwrap(), b"theirs");
    }
}
