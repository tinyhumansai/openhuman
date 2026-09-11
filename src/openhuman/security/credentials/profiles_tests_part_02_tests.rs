use super::*;

/// Sentry "Timed out waiting for auth profile lock" recovery: a lock
/// file that has been around for longer than `STALE_LOCK_AGE_MS` is
/// treated as leaked even if its recorded pid is still alive. This
/// covers the Windows AV / indexer case where `Drop::drop` on the
/// previous guard could not unlink the file and orphaned it with the
/// still-alive owner pid inside.
#[test]
fn clear_lock_if_stale_reclaims_lock_older_than_threshold_even_with_live_pid() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let lock_path = tmp.path().join(LOCK_FILENAME);
    std::fs::write(&lock_path, format!("pid={}\n", std::process::id())).unwrap();
    // Backdate the lock-file mtime well past STALE_LOCK_AGE_MS.
    let aged =
        std::time::SystemTime::now() - std::time::Duration::from_millis(STALE_LOCK_AGE_MS + 5_000);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("reopen lock for set_modified")
        .set_modified(aged)
        .expect("backdate lock mtime");

    assert!(
        store.clear_lock_if_stale(),
        "an aged lock with a live pid must be reclaimed (leaked-by-failed-unlink case)"
    );
    assert!(!lock_path.exists(), "stale lock should have been removed");
}

#[test]
fn clear_lock_if_stale_reclaims_aged_malformed_lock() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let lock_path = tmp.path().join(LOCK_FILENAME);
    std::fs::write(&lock_path, "garbage without a pid line\n").unwrap();
    let aged =
        std::time::SystemTime::now() - std::time::Duration::from_millis(STALE_LOCK_AGE_MS + 5_000);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("reopen lock for set_modified")
        .set_modified(aged)
        .expect("backdate lock mtime");

    assert!(
        store.clear_lock_if_stale(),
        "an aged malformed lock should be reclaimed"
    );
    assert!(!lock_path.exists());
}

/// Regression (init hang): a pidless lock left by a kill/crash mid-write must
/// be reclaimed after the short [`MALFORMED_LOCK_GRACE_MS`], NOT held for the
/// full [`STALE_LOCK_AGE_MS`]. Previously a fresh pidless lock made
/// `app_state_snapshot` (→ `acquire_lock`) block ~30s, stranding the user on
/// "Initializing OpenHuman" after a kill+reopen.
#[test]
fn clear_lock_if_stale_reclaims_pidless_lock_past_short_grace() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let lock_path = tmp.path().join(LOCK_FILENAME);
    std::fs::write(&lock_path, "garbage without a pid line\n").unwrap();
    // Past the malformed grace but far below the 30s stale-age threshold —
    // the old code would have left this in place and blocked ~30s.
    assert!(MALFORMED_LOCK_GRACE_MS + 500 < STALE_LOCK_AGE_MS);
    let aged = std::time::SystemTime::now()
        - std::time::Duration::from_millis(MALFORMED_LOCK_GRACE_MS + 500);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("reopen lock for set_modified")
        .set_modified(aged)
        .expect("backdate lock mtime");

    assert!(
        store.clear_lock_if_stale(),
        "a pidless lock past the short grace should be reclaimed without waiting STALE_LOCK_AGE_MS"
    );
    assert!(!lock_path.exists());
}

#[test]
fn lock_timeout_allows_fresh_leaked_locks_to_age_into_stale_reclaim() {
    assert!(
        LOCK_TIMEOUT_MS > STALE_LOCK_AGE_MS,
        "lock timeout must outlive stale-lock age so a fresh leaked lock can be reclaimed"
    );
    assert!(
        LOCK_TIMEOUT_MS - STALE_LOCK_AGE_MS >= 1_000,
        "timeout should leave at least one periodic stale recheck after the threshold"
    );
}

/// Sentry OPENHUMAN-TAURI-H8: when `OpenOptions::create_new` fails with
/// anything other than `AlreadyExists`, the error surfaced to Sentry
/// must embed the underlying `io::ErrorKind` and `raw_os_error()` so we
/// can tell which OS code is firing. Drive the wrapping helper directly
/// with a synthetic `io::Error` so the test is platform-independent and
/// doesn't depend on filesystem permissions (CI runs as root and bypasses
/// `chmod`).
#[test]
fn annotate_lock_create_failure_embeds_io_kind_and_os_code() {
    // Use each platform's native permission-denied code so the test exercises
    // the OS error that real production failures would carry. Rust does map
    // `from_raw_os_error(13)` to `PermissionDenied` on Windows too, but real
    // Windows `create_new` failures surface code 5 (ERROR_ACCESS_DENIED), and
    // running against the native code catches regressions in
    // `annotate_lock_create_failure`'s handling of the platform-specific
    // value.
    #[cfg(windows)]
    let raw_code = 5; // ERROR_ACCESS_DENIED
    #[cfg(not(windows))]
    let raw_code = 13; // EACCES

    let io_err = std::io::Error::from_raw_os_error(raw_code);
    let wrapped = annotate_lock_create_failure(anyhow::Error::new(io_err));
    let msg = format!("{wrapped:?}");

    assert!(
        msg.contains("Failed to create auth profile lock"),
        "stable top-level message missing: {msg}"
    );
    assert!(
        msg.contains("kind=Some(PermissionDenied)"),
        "context must include io::ErrorKind for Sentry diagnosis: {msg}"
    );
    assert!(
        msg.contains(&format!("os_code=Some({raw_code})")),
        "context must include raw OS code for Sentry diagnosis: {msg}"
    );
}

/// If somehow the chained error is not an `io::Error`, the wrapper must
/// still emit the stable top-level message with explicit `None` markers so
/// the Sentry fingerprint still splits cleanly (and we know to look
/// upstream for an io::Error that got dropped).
#[test]
fn annotate_lock_create_failure_handles_missing_io_error() {
    let wrapped = annotate_lock_create_failure(anyhow::anyhow!("synthetic"));
    let msg = format!("{wrapped:?}");

    assert!(msg.contains("Failed to create auth profile lock"), "{msg}");
    assert!(msg.contains("kind=None"), "{msg}");
    assert!(msg.contains("os_code=None"), "{msg}");
}

#[test]
fn auth_profile_kind_serde_roundtrip() {
    let json = serde_json::to_string(&AuthProfileKind::OAuth).unwrap();
    assert_eq!(json, "\"o-auth\""); // kebab-case
    let back: AuthProfileKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, AuthProfileKind::OAuth);

    let json = serde_json::to_string(&AuthProfileKind::Token).unwrap();
    assert_eq!(json, "\"token\"");
}

// ── Regression coverage for Sentry TAURI-RUST-92J / #3355 / #3364 ─────────
//
// `write_persisted_locked` retries transient Windows FS errors
// (`is_transient_fs_error` family — `ERROR_SHARING_VIOLATION` (32),
// `ERROR_ACCESS_DENIED` (5), `ERROR_DELETE_PENDING` (303), etc.) via
// `retry_with_backoff` on BOTH the `fs::write(tmp)` and the
// `fs::rename(tmp -> auth-profiles.json)` stages. Matches the sibling
// `.lock`-create retry that already closed OPENHUMAN-TAURI-H1 / H8 — the
// JSON `fs::write` + `fs::rename` path was the missing partial.
//
// Failure injection is now split per stage (`force_next_write_failures` and
// `force_next_rename_failures`) so each retry loop can be exercised in
// isolation. Originally a single shared counter, addressed in #3364 review
// where the rename retry path was line-covered but not behaviour-covered
// because the write stage drained every queued failure first.
//
// Each `#[cfg(test)]` consumer returns an error whose chain contains
// `__TEST_TRANSIENT__`, which `is_transient_fs_error` recognises as
// retryable on every platform (see `src/openhuman/util.rs`).

#[test]
fn write_stage_retries_one_shot_transient() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    // First write call returns the test sentinel; second runs the real
    // `fs::write` and succeeds. Rename stage is untouched.
    store.force_next_write_failures(1);

    let profile = AuthProfile::new_token("anthropic", "default", "tok-w1".into());
    store
        .upsert_profile(profile.clone(), true)
        .expect("retry should absorb the single write-stage transient");

    assert_eq!(store.remaining_forced_write_failures(), 0);
    assert_eq!(store.remaining_forced_rename_failures(), 0);

    let data = store.load().unwrap();
    assert!(data.profiles.contains_key(&profile.id));
}

#[test]
fn write_stage_absorbs_burst_of_transients() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    // 5 forced write failures — fewer than the retry budget
    // (PERSIST_RETRY_ATTEMPTS = 6), so the 6th attempt runs the real write
    // and succeeds. Covers the common "AV holds destination for a few
    // hundred ms" case which was the root cause of TAURI-RUST-92J.
    store.force_next_write_failures(5);

    let profile = AuthProfile::new_token("anthropic", "default", "tok-w-burst".into());
    store
        .upsert_profile(profile.clone(), true)
        .expect("retry must absorb a burst of write-stage transients within budget");

    assert_eq!(store.remaining_forced_write_failures(), 0);
    assert_eq!(store.remaining_forced_rename_failures(), 0);

    let data = store.load().unwrap();
    let loaded = data
        .profiles
        .get(&profile.id)
        .expect("profile must round-trip after retry");
    assert_eq!(loaded.token.as_deref(), Some("tok-w-burst"));
}

#[test]
fn write_stage_exhausts_retries_on_persistent_transient() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    // 6 forced failures — the full retry budget — so every attempt returns
    // the sentinel and `retry_with_backoff` ultimately surfaces the
    // failed-after-N-attempts error. Genuinely unrecoverable failures still
    // reach Sentry as honest signal; not a noise-suppression layer.
    store.force_next_write_failures(6);

    let profile = AuthProfile::new_token("anthropic", "default", "tok-w2".into());
    let err = store
        .upsert_profile(profile, true)
        .expect_err("persistent write-stage transient must exhaust retries and surface as Err");

    let chain = format!("{err:?}");
    assert!(
        chain.contains("Failed to write temporary auth profile file"),
        "outer with_context must be preserved for Sentry fingerprint stability: {chain}"
    );
    assert!(
        chain.contains("write auth profile tmp failed after"),
        "retry helper must annotate the exhausted attempts count: {chain}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Disk-full auth-profile read resilience (Sentry TAURI-RUST-4SZ)
// ─────────────────────────────────────────────────────────────────────────

/// When the exclusive lock can't be created because the filesystem is full,
/// the READ path must degrade to a lock-free read of the existing store
/// rather than failing — otherwise `app_state_snapshot` strands the UI and
/// floods Sentry once per poll. Writers publish atomically, so the lock-free
/// read is consistent.
#[tokio::test]
async fn load_falls_back_to_lock_free_read_when_disk_full() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);

    let profile = AuthProfile::new_token("openai-codex", "default", "tok-abc".into());
    store.upsert_profile(profile.clone(), true).unwrap();

    // Next acquire_lock simulates a StorageFull (ENOSPC) lock-create failure.
    store.force_next_lock_unwritable();

    // load() must still return the persisted profile via the read-only fallback.
    let data = store
        .load()
        .expect("load must degrade to lock-free read on disk-full");
    assert!(
        data.profiles.contains_key(&profile.id),
        "lock-free fallback must still surface the existing session profile"
    );

    // The flag is one-shot: the next load takes the lock normally.
    let again = store
        .load()
        .expect("subsequent load takes the lock normally");
    assert!(again.profiles.contains_key(&profile.id));
}

/// The lock-free read path must return the same resolved data as the locked
/// path for a healthy store — it differs only in that it skips the
/// opportunistic on-disk rewrite, never in what it returns.
#[tokio::test]
async fn load_unlocked_readonly_matches_locked_load() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);

    let profile = AuthProfile::new_token("slack", "default", "tok-xyz".into());
    store.upsert_profile(profile.clone(), true).unwrap();

    let locked = store.load().unwrap();
    let unlocked = store.load_unlocked_readonly().unwrap();

    assert_eq!(
        locked.profiles.keys().collect::<Vec<_>>(),
        unlocked.profiles.keys().collect::<Vec<_>>(),
        "lock-free read must resolve the same profile set as the locked load"
    );
    assert_eq!(locked.active_profiles, unlocked.active_profiles);
}

/// Polarity guard for the read-path fallback predicate: only genuine
/// filesystem-unwritable conditions (disk full / read-only mount) degrade the
/// read; lock contention and unrelated errors must still propagate.
#[test]
fn is_lock_create_unwritable_fs_polarity() {
    let storage_full = annotate_lock_create_failure(
        anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::StorageFull))
            .context("open lock file"),
    );
    assert!(is_lock_create_unwritable_fs(&storage_full));

    let read_only = annotate_lock_create_failure(
        anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ReadOnlyFilesystem))
            .context("open lock file"),
    );
    assert!(is_lock_create_unwritable_fs(&read_only));

    // Lock contention / timeout — must NOT degrade the read.
    let timeout = anyhow::anyhow!("Timed out waiting for auth profile lock");
    assert!(!is_lock_create_unwritable_fs(&timeout));

    // A different FS error (permissions) is a real problem — keep it visible.
    let perm = annotate_lock_create_failure(
        anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            .context("open lock file"),
    );
    assert!(!is_lock_create_unwritable_fs(&perm));
}

/// Drift guard coupling the Sentry `DiskFull` classifier to the ACTUAL
/// producer output. `annotate_lock_create_failure` embeds the `io::ErrorKind`
/// debug name (`StorageFull`) instead of the io Display, and at the RPC
/// boundary the error is flattened single-line (`{}`), so the inner "no space
/// left on device" text never reaches the classifier. This asserts the
/// rendered producer string both (a) lacks that legacy anchor and (b) still
/// classifies as DiskFull — so a future format!() / std rename fails CI here
/// instead of silently re-leaking the flood.
#[test]
fn disk_full_lock_failure_string_classifies_as_disk_full() {
    use crate::core::observability::{expected_error_kind, ExpectedErrorKind};

    let err = annotate_lock_create_failure(
        anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::StorageFull))
            .context("open lock file"),
    );
    // The single-line Display form is what production flattens to.
    let rendered = format!("{err}");

    assert!(
        !rendered.to_lowercase().contains("no space left on device"),
        "outer-only render must NOT carry the legacy anchor (that's the whole bug): {rendered}"
    );
    assert_eq!(
        expected_error_kind(&rendered),
        Some(ExpectedErrorKind::DiskFull),
        "producer output must classify as DiskFull via the StorageFull anchor: {rendered}"
    );
}

#[test]
fn rename_stage_retries_one_shot_transient() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    // No write-stage injection — write runs clean on the first attempt.
    // The first rename attempt returns the sentinel; the second succeeds.
    // This is the path the headline of PR #3364 was about: previously the
    // shared-counter design left this loop with line coverage but no
    // behaviour coverage.
    store.force_next_rename_failures(1);

    let profile = AuthProfile::new_token("anthropic", "default", "tok-r1".into());
    store
        .upsert_profile(profile.clone(), true)
        .expect("retry should absorb the single rename-stage transient");

    assert_eq!(store.remaining_forced_write_failures(), 0);
    assert_eq!(store.remaining_forced_rename_failures(), 0);

    let data = store.load().unwrap();
    assert!(data.profiles.contains_key(&profile.id));

    // Successful rename consumes the tmp; directory should hold only the
    // final `auth-profiles.json` (plus the `.lock`, if still present from
    // the operation). No orphaned tmp files even after retry.
    let parent = store.path().parent().unwrap();
    let leaked: Vec<_> = std::fs::read_dir(parent)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("auth-profiles.json.tmp.")
        })
        .collect();
    assert!(
        leaked.is_empty(),
        "successful rename must consume the tmp, not orphan it: {leaked:?}"
    );
}

#[test]
fn rename_stage_exhausts_retries_and_cleans_up_tmp() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    // Full retry budget on the rename stage — every attempt returns the
    // sentinel, so `retry_with_backoff` surfaces failed-after-N-attempts.
    // This is the test the shared-counter design could not express — the
    // write stage previously drained the queue before the rename closure
    // ever ran, so the rename's outer `with_context` ("Failed to replace
    // auth profile store") was unreachable from a green test.
    store.force_next_rename_failures(6);

    let profile = AuthProfile::new_token("anthropic", "default", "tok-r2".into());
    let err = store
        .upsert_profile(profile, true)
        .expect_err("persistent rename-stage transient must exhaust retries and surface as Err");

    let chain = format!("{err:?}");
    assert!(
        chain.contains("Failed to replace auth profile store"),
        "rename-stage outer with_context must be preserved for Sentry fingerprint stability: {chain}"
    );
    assert!(
        chain.contains("replace auth profile store failed after"),
        "retry helper must annotate the exhausted attempts count for the rename stage: {chain}"
    );

    // Best-effort tmp cleanup: the rename retry exhausted, but the
    // best-effort `fs::remove_file(&tmp_path)` in `write_persisted_locked`
    // should have removed the orphaned `auth-profiles.json.tmp.{pid}.{nanos}`.
    // (Pre-#3364-followup this test would fail because the tmp was leaked
    // on every sustained-failure poll.)
    let parent = store.path().parent().unwrap();
    let leaked: Vec<_> = std::fs::read_dir(parent)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("auth-profiles.json.tmp.")
        })
        .collect();
    assert!(
        leaked.is_empty(),
        "rename exhaustion must trigger best-effort tmp cleanup; leaked: {leaked:?}"
    );
}
