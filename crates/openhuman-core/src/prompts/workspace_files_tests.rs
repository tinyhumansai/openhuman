use super::*;

#[test]
fn a_new_builtin_replaces_an_untouched_file_and_keeps_an_edited_one() {
    // A release changes the bundled SOUL.md. A workspace whose copy still
    // matches the previous built-in gets the new one; a workspace where the
    // user edited it keeps the edit. Both record the new built-in's hash.
    fn hash(content: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
    let fresh = tempfile::tempdir().unwrap();
    sync_workspace_file(fresh.path(), "SOUL.md");
    let current = std::fs::read_to_string(fresh.path().join("SOUL.md")).unwrap();
    let current_hash = std::fs::read_to_string(fresh.path().join(".SOUL.md.builtin-hash")).unwrap();

    for (on_disk, expected) in [
        ("previous built-in SOUL.md", current.as_str()),
        ("the user's own SOUL.md", "the user's own SOUL.md"),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("SOUL.md"), on_disk).unwrap();
        std::fs::write(
            workspace.path().join(".SOUL.md.builtin-hash"),
            hash("previous built-in SOUL.md"),
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
