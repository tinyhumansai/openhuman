use super::*;

#[test]
fn a_new_builtin_replaces_an_untouched_file_and_keeps_an_edited_one() {
    // A release changes the bundled SOUL.md. A workspace whose copy still
    // matches the previous built-in gets the new one; a workspace where the
    // user edited it keeps the edit. Both record the new built-in's hash.
    let fresh = tempfile::tempdir().unwrap();
    sync_workspace_file(fresh.path(), "SOUL.md");
    let current_hash = std::fs::read_to_string(fresh.path().join(".SOUL.md.builtin-hash")).unwrap();

    for (on_disk, expected) in [
        ("previous built-in SOUL.md", "previous built-in SOUL.md"),
        ("the user's own SOUL.md", "the user's own SOUL.md"),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("SOUL.md"), on_disk).unwrap();
        std::fs::write(
            workspace.path().join(".SOUL.md.builtin-hash"),
            legacy_hash("previous built-in SOUL.md"),
        )
        .unwrap();

        sync_workspace_file(workspace.path(), "SOUL.md");

        assert_eq!(
            std::fs::read_to_string(workspace.path().join("SOUL.md")).unwrap(),
            expected,
            "on disk before the upgrade: {on_disk:?}"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.path().join(".SOUL.md.builtin-hash")).unwrap(),
            current_hash
        );
    }
}

/// The sidecar format every sync must leave behind: lowercase hex SHA-256 of
/// the built-in. Computed here independently of the code under test.
fn sha256_hex(content: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(content.as_bytes()))
}

/// The pre-#322 sidecar format: `DefaultHasher`, whose output may change with
/// any Rust release.
fn legacy_hash(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[test]
fn the_builtin_hash_sidecar_is_a_stable_sha256() {
    // `DefaultHasher` is not stable across Rust releases, so a toolchain bump
    // could make an edited file look like an untouched built-in. The sidecar
    // must be a hash whose output is fixed forever.
    let workspace = tempfile::tempdir().unwrap();
    sync_workspace_file(workspace.path(), "SOUL.md");

    let sidecar = std::fs::read_to_string(workspace.path().join(".SOUL.md.builtin-hash")).unwrap();
    assert_eq!(
        sidecar,
        sha256_hex(default_workspace_file_content("SOUL.md"))
    );

    // A second sync with nothing changed leaves the file and sidecar alone.
    sync_workspace_file(workspace.path(), "SOUL.md");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".SOUL.md.builtin-hash")).unwrap(),
        sidecar
    );
}

#[test]
fn a_legacy_sidecar_is_migrated_without_authorizing_an_overwrite() {
    // A workspace synced before #322: its SOUL.md is still the previous
    // built-in and the sidecar holds that built-in's `DefaultHasher` hash.
    // Legacy hashes are not trusted to distinguish an untouched file from a
    // colliding user edit. Preserve disk content and migrate the sidecar.
    let workspace = tempfile::tempdir().unwrap();
    let soul = workspace.path().join("SOUL.md");
    let sidecar = workspace.path().join(".SOUL.md.builtin-hash");
    std::fs::write(&soul, "previous built-in SOUL.md").unwrap();
    std::fs::write(&sidecar, legacy_hash("previous built-in SOUL.md")).unwrap();

    sync_workspace_file(workspace.path(), "SOUL.md");

    let builtin = default_workspace_file_content("SOUL.md");
    assert_eq!(
        std::fs::read_to_string(&soul).unwrap(),
        "previous built-in SOUL.md"
    );
    assert_eq!(
        std::fs::read_to_string(&sidecar).unwrap(),
        sha256_hex(builtin)
    );
}

#[test]
fn a_user_edited_file_with_a_legacy_sidecar_is_never_overwritten() {
    // Same pre-#322 workspace, but the user edited SOUL.md. Neither the
    // format change nor a legacy sidecar from another toolchain may cost
    // them the edit; the sidecar still moves to SHA-256.
    let builtin = default_workspace_file_content("SOUL.md");
    for stored in [
        legacy_hash("previous built-in SOUL.md"),
        // A collision with the edited disk content must not authorize it.
        legacy_hash("the user's own SOUL.md"),
        // A sidecar written by a different toolchain's `DefaultHasher`.
        "0123456789abcdef".to_string(),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let soul = workspace.path().join("SOUL.md");
        let sidecar = workspace.path().join(".SOUL.md.builtin-hash");
        std::fs::write(&soul, "the user's own SOUL.md").unwrap();
        std::fs::write(&sidecar, &stored).unwrap();

        sync_workspace_file(workspace.path(), "SOUL.md");

        assert_eq!(
            std::fs::read_to_string(&soul).unwrap(),
            "the user's own SOUL.md",
            "stored sidecar: {stored:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&sidecar).unwrap(),
            sha256_hex(builtin)
        );
    }
}

#[test]
fn the_truncation_marker_names_the_real_read_tool() {
    // The model is told how to recover a truncated file; the tool it is told
    // to call must be one it has.
    let mut inline = String::new();
    inject_inline_content(&mut inline, "NOTES.md", "abcdefghij", 4);
    assert!(inline.contains("use `file_read` for full file"), "{inline}");

    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("NOTES.md"), "abcdefghij").unwrap();
    let mut from_disk = String::new();
    inject_workspace_file_capped(&mut from_disk, workspace.path(), "NOTES.md", 4);
    assert!(
        from_disk.contains("use `file_read` for full file"),
        "{from_disk}"
    );
}
