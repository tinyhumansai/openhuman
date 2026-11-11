use super::*;

#[test]
fn generate_token_produces_64_hex_chars() {
    let t = generate_token();
    assert_eq!(t.len(), 64, "256 bits → 64 hex chars");
    assert!(t.chars().all(|c| c.is_ascii_hexdigit()), "must be hex");
}

#[test]
fn generate_token_is_not_constant() {
    assert_ne!(generate_token(), generate_token());
}

#[test]
fn write_and_read_token_roundtrips() {
    let tmp = std::env::temp_dir().join(format!("core-auth-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("core.token");
    let token = "cafebabe1234567890abcdef0123456789abcdef0123456789abcdef01234567";
    write_token_file(&path, token).unwrap();
    let back = std::fs::read_to_string(&path).unwrap();
    assert_eq!(back, token);
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn bearer_matches_rejects_empty_supplied() {
    let expected = "cafebabe";
    assert!(!bearer_matches("", expected));
}

#[test]
fn bearer_matches_rejects_mismatch() {
    assert!(!bearer_matches("deadbeef", "cafebabe"));
}

#[test]
fn bearer_matches_rejects_prefix_match() {
    assert!(!bearer_matches("cafeba", "cafebabe"));
}

#[test]
fn bearer_matches_accepts_exact() {
    assert!(bearer_matches("cafebabe", "cafebabe"));
}

#[test]
fn verify_bearer_token_returns_false_when_token_uninitialized() {
    // RPC_TOKEN is a process-global OnceLock; on a fresh test binary it
    // may already be set by another test that ran first, so we cannot
    // assert the uninitialized branch here without process isolation.
    // We can however confirm that an empty supplied value is always
    // rejected, which exercises the second-leg invariant.
    assert!(!verify_bearer_token(""));
}

#[test]
fn init_rpc_token_with_value_rejects_empty() {
    // Trimmed-empty values must error rather than seed an empty bearer.
    assert!(init_rpc_token_with_value("").is_err());
    assert!(init_rpc_token_with_value("   ").is_err());
}

/// `init_rpc_token_with_value` populates the same `RPC_TOKEN` OnceLock
/// that `get_rpc_token` reads — i.e. the in-memory handoff path produces
/// the bearer everyone else (HTTP middleware, Socket.IO verifier,
/// approval-gate session_id) reads from. We can't deterministically
/// assert the *value* set here (the OnceLock may already be seeded by a
/// sibling test that ran first in the same binary), but we can assert
/// the OnceLock is initialised after this call returns Ok, and that the
/// helper is idempotent.
#[test]
fn init_rpc_token_with_value_seeds_and_is_idempotent() {
    // First call: either we seed, or a sibling test already did. Either
    // way the helper must return Ok and leave `get_rpc_token` populated.
    let token = "cafebabe1234567890abcdef0123456789abcdef0123456789abcdef01234567";
    init_rpc_token_with_value(token).expect("seed succeeds");
    assert!(
        get_rpc_token().is_some(),
        "after init_rpc_token_with_value, get_rpc_token must return Some"
    );
    // Second call is a no-op (matching init_rpc_token semantics) — must
    // not error, must not flip the in-memory value.
    let before = get_rpc_token().map(str::to_string);
    init_rpc_token_with_value("a-different-value-that-must-be-ignored")
        .expect("idempotent re-init succeeds");
    let after = get_rpc_token().map(str::to_string);
    assert_eq!(
        before, after,
        "second init_rpc_token_with_value must not flip the in-memory bearer"
    );
}

#[cfg(unix)]
#[test]
fn token_file_has_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = std::env::temp_dir().join(format!("core-auth-perms-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("core.token");
    write_token_file(&path, "abc").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "token file must be 0o600");
    std::fs::remove_dir_all(&tmp).ok();
}

#[cfg(unix)]
#[test]
fn token_file_tightens_preexisting_loose_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    // A token file left world-readable (an older build, a copy, a restore)
    // must come back owner-only when the token is rewritten.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("core.token");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_token_file(&path, "new").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "rewritten token kept mode {:o}",
        mode & 0o777
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
}

#[cfg(unix)]
#[test]
fn a_descriptor_open_on_the_old_token_never_sees_the_new_one() {
    use std::io::{Read as _, Seek as _};

    // Someone who opened the old (loose) file keeps that descriptor after a
    // chmod; the rewrite must land on a new inode so it reads only the old token.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("core.token");
    std::fs::write(&path, "old").unwrap();
    let mut held = std::fs::File::open(&path).unwrap();
    write_token_file(&path, "new").unwrap();
    let mut seen = String::new();
    held.rewind().unwrap();
    held.read_to_string(&mut seen).unwrap();
    assert_eq!(seen, "old");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
}
