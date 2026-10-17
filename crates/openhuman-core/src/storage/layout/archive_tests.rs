use super::*;
use crate::storage::layout::test_fixtures::{tree, write as put};
use tempfile::TempDir;

fn sources(dir: &Path) -> Vec<Source> {
    vec![
        Source {
            rel: "skills".into(),
            action: Action::Move,
            at: dir.join("skills"),
        },
        Source {
            rel: "memory/jobs.json".into(),
            action: Action::Delete,
            at: dir.join("jobs.json"),
        },
    ]
}

fn source_tree() -> TempDir {
    let dir = TempDir::new().unwrap();
    put(
        &dir.path().join("skills/s1/SKILL.md"),
        "UNIQUE-CONTENT-1234 skill body",
    );
    put(&dir.path().join("skills/s2/notes.txt"), "second");
    std::fs::create_dir_all(dir.path().join("skills/empty")).unwrap();
    put(&dir.path().join("jobs.json"), "{\"a\":1}");
    #[cfg(unix)]
    std::os::unix::fs::symlink("s1/SKILL.md", dir.path().join("skills/link")).unwrap();
    dir
}

#[test]
fn an_archive_verifies_against_its_own_manifest() {
    let src = source_tree();
    let out = TempDir::new().unwrap();
    let archive = out.path().join("backups/u1-test.tar.zst");
    let written = write(&archive, "u1", 1_700_000_000_000, &sources(src.path())).unwrap();
    let verified = verify(&archive).unwrap();
    assert_eq!(written, verified);
    let paths: Vec<_> = verified.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"workspace/skills/s1/SKILL.md"), "{paths:?}");
    assert!(paths.contains(&"workspace/memory/jobs.json"), "{paths:?}");
    let skill = verified
        .files
        .iter()
        .find(|f| f.path.ends_with("s1/SKILL.md"))
        .unwrap();
    assert_eq!(skill.size, "UNIQUE-CONTENT-1234 skill body".len() as u64);
    assert_eq!(skill.sha256.len(), 64);
    assert!(
        sidecar_path(&archive).is_file(),
        "the manifest is also written beside the archive"
    );
    #[cfg(unix)]
    assert!(verified.files.iter().any(|f| f.kind == "symlink"));
}

#[test]
fn a_corrupted_entry_fails_verification() {
    let src = source_tree();
    let out = TempDir::new().unwrap();
    let archive = out.path().join("a.tar.zst");
    write(&archive, "u1", 0, &sources(src.path())).unwrap();
    // Alter one file's bytes inside the tar stream (same length, so the tar
    // stays well-formed and only the manifest hash can notice) and re-compress.
    let mut tar_bytes = zstd::decode_all(File::open(&archive).unwrap()).unwrap();
    let at = tar_bytes
        .windows(b"UNIQUE-CONTENT".len())
        .position(|w| w == b"UNIQUE-CONTENT")
        .expect("the file's bytes are in the tar stream");
    tar_bytes[at] = b'X';
    std::fs::write(&archive, zstd::encode_all(tar_bytes.as_slice(), 1).unwrap()).unwrap();
    let error = verify(&archive).unwrap_err().to_string();
    assert!(
        error.contains("SKILL.md") && error.contains("does not match"),
        "{error}"
    );
}

#[test]
fn a_truncated_archive_fails_verification() {
    let src = source_tree();
    let out = TempDir::new().unwrap();
    let archive = out.path().join("a.tar.zst");
    write(&archive, "u1", 0, &sources(src.path())).unwrap();
    let bytes = std::fs::read(&archive).unwrap();
    std::fs::write(&archive, &bytes[..bytes.len() / 2]).unwrap();
    assert!(matches!(verify(&archive), Err(LayoutError::Verify(_))));
}

#[test]
fn restore_puts_every_file_back_under_workspace() {
    let src = source_tree();
    let out = TempDir::new().unwrap();
    let archive = out.path().join("a.tar.zst");
    write(&archive, "u1", 0, &sources(src.path())).unwrap();
    let profile = TempDir::new().unwrap();
    let manifest = restore(&archive, profile.path()).unwrap();
    assert_eq!(manifest.entries.len(), 2);
    let restored = tree(&profile.path().join("workspace"));
    let original = tree(src.path());
    assert_eq!(
        restored["skills/s1/SKILL.md"],
        original["skills/s1/SKILL.md"]
    );
    assert_eq!(
        restored["skills/s2/notes.txt"],
        original["skills/s2/notes.txt"]
    );
    assert_eq!(restored["memory/jobs.json"], original["jobs.json"]);
    assert!(restored.contains_key("skills/empty"));
}

#[test]
fn restore_of_a_corrupt_archive_extracts_nothing() {
    let src = source_tree();
    let out = TempDir::new().unwrap();
    let archive = out.path().join("a.tar.zst");
    write(&archive, "u1", 0, &sources(src.path())).unwrap();
    let mut bytes = std::fs::read(&archive).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&archive, bytes).unwrap();
    let profile = TempDir::new().unwrap();
    assert!(restore(&archive, profile.path()).is_err());
    assert!(std::fs::read_dir(profile.path()).unwrap().next().is_none());
}

#[test]
fn a_failed_write_leaves_no_partial_archive() {
    let out = TempDir::new().unwrap();
    let archive = out.path().join("a.tar.zst");
    let missing = vec![Source {
        rel: "x".into(),
        action: Action::Delete,
        at: out.path().join("absent"),
    }];
    assert!(write(&archive, "u1", 0, &missing).is_err());
    assert!(!archive.exists());
}
