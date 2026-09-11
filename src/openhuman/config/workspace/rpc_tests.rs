use super::*;
use tempfile::tempdir;

#[test]
fn read_returns_bundled_default_when_file_missing() {
    let tmp = tempdir().unwrap();
    let outcome = read_workspace_file(tmp.path(), "SOUL.md").expect("read should succeed");
    let file = outcome.value;
    assert!(file.is_default, "missing file should report the default");
    assert!(!file.contents.trim().is_empty());
    assert_eq!(file.filename, "SOUL.md");
    assert_eq!(
        file.contents,
        bundled_default_contents("SOUL.md").unwrap(),
        "default read must match the bundled prompt"
    );
}

#[test]
fn read_returns_on_disk_contents_when_present() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("SOUL.md"), "custom soul").unwrap();
    let file = read_workspace_file(tmp.path(), "SOUL.md")
        .expect("read ok")
        .value;
    assert!(!file.is_default);
    assert_eq!(file.contents, "custom soul");
}

#[test]
fn read_refuses_oversize_file_on_disk() {
    let tmp = tempdir().unwrap();
    let huge = "a".repeat((MAX_WORKSPACE_FILE_BYTES + 1) as usize);
    std::fs::write(tmp.path().join("SOUL.md"), &huge).unwrap();
    let err = read_workspace_file(tmp.path(), "SOUL.md").unwrap_err();
    assert!(err.contains("too large"), "unexpected error: {err}");
}

#[test]
fn read_accepts_file_exactly_at_the_size_limit() {
    let tmp = tempdir().unwrap();
    let at_limit = "a".repeat(MAX_WORKSPACE_FILE_BYTES as usize);
    std::fs::write(tmp.path().join("SOUL.md"), &at_limit).unwrap();
    let file = read_workspace_file(tmp.path(), "SOUL.md")
        .expect("exactly-at-limit read should succeed")
        .value;
    assert_eq!(file.contents.len(), MAX_WORKSPACE_FILE_BYTES as usize);
    assert!(!file.is_default);
}

#[test]
fn read_rejects_non_utf8_file() {
    let tmp = tempdir().unwrap();
    // A lone 0xFF byte is never valid UTF-8.
    std::fs::write(tmp.path().join("SOUL.md"), [0xff_u8, 0xfe, 0xfd]).unwrap();
    let err = read_workspace_file(tmp.path(), "SOUL.md").unwrap_err();
    assert!(err.contains("UTF-8"), "unexpected error: {err}");
}

#[test]
fn write_then_read_round_trips() {
    let tmp = tempdir().unwrap();
    let written = write_workspace_file(tmp.path(), "SOUL.md", "You are calm and concise.")
        .expect("write ok")
        .value;
    assert!(!written.is_default);
    assert_eq!(written.contents, "You are calm and concise.");

    let read = read_workspace_file(tmp.path(), "SOUL.md")
        .expect("read ok")
        .value;
    assert_eq!(read.contents, "You are calm and concise.");
    assert!(!read.is_default);
}

#[test]
fn write_creates_workspace_dir_if_missing() {
    let tmp = tempdir().unwrap();
    let nested = tmp.path().join("does/not/exist/yet");
    let written = write_workspace_file(&nested, "IDENTITY.md", "id")
        .expect("write should create the dir")
        .value;
    assert_eq!(written.contents, "id");
    assert!(nested.join("IDENTITY.md").is_file());
}

#[test]
fn reset_restores_bundled_default() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("SOUL.md"), "corrupted").unwrap();
    let reset = reset_workspace_file(tmp.path(), "SOUL.md")
        .expect("reset ok")
        .value;
    assert!(reset.is_default);
    assert_eq!(reset.contents, bundled_default_contents("SOUL.md").unwrap());
    let on_disk = std::fs::read_to_string(tmp.path().join("SOUL.md")).unwrap();
    assert_eq!(on_disk, bundled_default_contents("SOUL.md").unwrap());
}

#[test]
fn non_allowlisted_filename_is_rejected_for_every_op() {
    let tmp = tempdir().unwrap();
    for name in ["secrets.txt", "../escape.md", "MEMORY.md", "soul.md"] {
        assert!(read_workspace_file(tmp.path(), name).is_err());
        assert!(write_workspace_file(tmp.path(), name, "x").is_err());
        assert!(reset_workspace_file(tmp.path(), name).is_err());
    }
    // The rejection must not have written anything to disk.
    assert!(!tmp.path().join("MEMORY.md").exists());
}

#[test]
fn write_rejects_oversize_contents() {
    let tmp = tempdir().unwrap();
    let huge = "a".repeat((MAX_WORKSPACE_FILE_BYTES + 1) as usize);
    let err = write_workspace_file(tmp.path(), "SOUL.md", &huge).unwrap_err();
    assert!(err.contains("limit"), "unexpected error: {err}");
    assert!(
        !tmp.path().join("SOUL.md").exists(),
        "oversize write must not touch disk"
    );
}

#[test]
fn write_accepts_contents_at_the_size_limit() {
    let tmp = tempdir().unwrap();
    let at_limit = "a".repeat(MAX_WORKSPACE_FILE_BYTES as usize);
    let written = write_workspace_file(tmp.path(), "SOUL.md", &at_limit)
        .expect("exactly-at-limit write should succeed")
        .value;
    assert_eq!(written.contents.len(), MAX_WORKSPACE_FILE_BYTES as usize);
}
