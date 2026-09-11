// `super` is the `tests` module, whose own `use super::*` re-exports the
// private items of `profiles` -- including `write_owner_only`.
use super::*;

#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

/// The property this change is actually about, asserted on the store rather
/// than on the helper: after a real save, the credential file on disk is not
/// readable by anyone but its owner.
///
/// The helper tests below cannot see the wiring — they call `write_owner_only`
/// directly, so swapping the call site back to `fs::write` leaves them green.
/// This one goes through `upsert_profile`, which writes the tmp and renames it
/// over the store, so it covers the mode *and* the fact that `rename` carries
/// it across.
///
/// Compiled on every platform so the store API usage is type-checked
/// everywhere; only the assertion is unix-gated, since Windows has no mode to
/// look at.
#[test]
fn a_saved_store_file_is_owner_only() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let profile = AuthProfile::new_oauth(
        "openai-codex",
        "default",
        TokenSet {
            access_token: "access-123".into(),
            refresh_token: Some("refresh-123".into()),
            id_token: None,
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(profile, true).unwrap();

    let path = store.path();
    assert!(path.exists(), "the save must have produced a store file");

    #[cfg(unix)]
    assert_eq!(
        mode_of(&path),
        0o600,
        "the credential store must not be group/world readable"
    );
}

#[cfg(unix)]
#[test]
fn a_new_credential_file_is_owner_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("auth-profiles.json.tmp");
    write_owner_only(&path, b"{}").expect("write");
    assert_eq!(
        mode_of(&path),
        0o600,
        "credential store must not be group/world readable"
    );
}

#[cfg(unix)]
#[test]
fn a_leftover_world_readable_tmp_is_repaired() {
    // `.mode()` only applies at creation, so an interrupted save that left a
    // 0644 tmp behind would otherwise keep it and carry it through `rename`.
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("auth-profiles.json.tmp");
    std::fs::write(&path, b"stale").expect("seed");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert_eq!(mode_of(&path), 0o644, "precondition");

    write_owner_only(&path, b"{}").expect("write");
    assert_eq!(mode_of(&path), 0o600);
}

#[test]
fn the_contents_are_written_and_truncated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("auth-profiles.json.tmp");
    write_owner_only(&path, b"aaaaaaaaaaaaaaaaaaaa").expect("write long");
    write_owner_only(&path, b"bb").expect("write short");
    assert_eq!(
        std::fs::read(&path).expect("read"),
        b"bb",
        "stale bytes must not survive"
    );
}
