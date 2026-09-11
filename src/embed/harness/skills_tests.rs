use super::*;

/// Write a minimal skill bundle at `dir` and return it.
fn bundle(dir: &Path, manifest: &str, body: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("create bundle dir");
    std::fs::write(dir.join(manifest), body).expect("write manifest");
    dir.to_path_buf()
}

#[test]
fn bundles_are_copied_into_the_workspace_skills_root() {
    let src = tempfile::tempdir().expect("src");
    let ws = tempfile::tempdir().expect("ws");
    bundle(&src.path().join("alpha"), "SKILL.md", "# alpha");
    bundle(&src.path().join("beta"), "WORKFLOW.md", "# beta");

    let installed = install(src.path(), ws.path()).expect("install");
    assert_eq!(installed, 2);

    // `<workspace>/skills` is the legacy skill root — the one root a harness
    // controls that discovery scans without a trust marker.
    assert!(ws.path().join("skills/alpha/SKILL.md").is_file());
    assert!(ws.path().join("skills/beta/WORKFLOW.md").is_file());
}

#[test]
fn a_directory_that_is_itself_a_bundle_is_copied_under_its_own_name() {
    let src = tempfile::tempdir().expect("src");
    let ws = tempfile::tempdir().expect("ws");
    let solo = bundle(&src.path().join("solo"), "SKILL.md", "# solo");

    let installed = install(&solo, ws.path()).expect("install");
    assert_eq!(installed, 1);
    assert!(ws.path().join("skills/solo/SKILL.md").is_file());
}

#[test]
fn nested_bundle_files_come_along() {
    let src = tempfile::tempdir().expect("src");
    let ws = tempfile::tempdir().expect("ws");
    let dir = src.path().join("alpha");
    bundle(&dir, "SKILL.md", "# alpha");
    std::fs::create_dir_all(dir.join("scripts")).expect("mkdir");
    std::fs::write(dir.join("scripts/run.sh"), "echo hi").expect("write");

    install(src.path(), ws.path()).expect("install");
    assert_eq!(
        std::fs::read_to_string(ws.path().join("skills/alpha/scripts/run.sh")).expect("read"),
        "echo hi"
    );
}

#[test]
fn a_directory_without_a_manifest_is_not_a_bundle() {
    let src = tempfile::tempdir().expect("src");
    let ws = tempfile::tempdir().expect("ws");
    std::fs::create_dir_all(src.path().join("notaskill")).expect("mkdir");
    std::fs::write(src.path().join("notaskill/readme.txt"), "hi").expect("write");

    assert_eq!(install(src.path(), ws.path()).expect("install"), 0);
    assert!(!ws.path().join("skills/notaskill").exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_bundle_is_skipped_not_followed() {
    // Discovery refuses symlinked bundle dirs on purpose (`scan_root_inner`
    // uses `file_type()` precisely so a link cannot be loaded as a skill).
    // Copying one in would smuggle past a control that exists because this root
    // is scanned with no trust marker.
    let src = tempfile::tempdir().expect("src");
    let ws = tempfile::tempdir().expect("ws");
    let outside = tempfile::tempdir().expect("outside");
    bundle(&outside.path().join("evil"), "SKILL.md", "# evil");

    std::os::unix::fs::symlink(outside.path().join("evil"), src.path().join("evil"))
        .expect("symlink");

    assert_eq!(install(src.path(), ws.path()).expect("install"), 0);
    assert!(!ws.path().join("skills/evil").exists());
}

#[cfg(unix)]
#[test]
fn a_symlink_inside_a_bundle_is_skipped_not_followed() {
    let src = tempfile::tempdir().expect("src");
    let ws = tempfile::tempdir().expect("ws");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("secret"), "token").expect("write");

    let dir = src.path().join("alpha");
    bundle(&dir, "SKILL.md", "# alpha");
    std::os::unix::fs::symlink(outside.path().join("secret"), dir.join("leak")).expect("symlink");

    install(src.path(), ws.path()).expect("install");
    assert!(ws.path().join("skills/alpha/SKILL.md").is_file());
    assert!(
        !ws.path().join("skills/alpha/leak").exists(),
        "a symlink inside a bundle must not be copied through"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_manifest_does_not_make_a_directory_a_bundle() {
    // `is_bundle` must not trust symlinks: `is_file()` follows a link, so a
    // manifest that is itself a symlink would count the directory as a bundle
    // even though `copy_tree` skips symlinks — installing a directory with no
    // manifest that discovery then ignores (a silent-absence failure). A
    // symlinked manifest must be rejected like a symlinked bundle dir.
    let src = tempfile::tempdir().expect("src");
    let _ws = tempfile::tempdir().expect("ws");
    let outside = tempfile::tempdir().expect("outside");
    let dir = src.path().join("tricky");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(outside.path().join("SKILL.md"), "# real").expect("write");
    std::os::unix::fs::symlink(outside.path().join("SKILL.md"), dir.join("SKILL.md"))
        .expect("symlink manifest");

    assert!(
        !is_bundle(&dir),
        "a symlinked manifest must not be treated as a bundle"
    );
}

#[test]
fn a_missing_skills_dir_is_a_clear_error_not_a_silent_skip() {
    let ws = tempfile::tempdir().expect("ws");
    let err = install(std::path::Path::new("/nonexistent/skills"), ws.path())
        .expect_err("a missing directory is a caller mistake");
    assert!(matches!(err, HarnessError::Invalid(_)));
}

#[test]
fn an_empty_skills_dir_is_allowed() {
    let src = tempfile::tempdir().expect("src");
    let ws = tempfile::tempdir().expect("ws");
    assert_eq!(install(src.path(), ws.path()).expect("install"), 0);
}
