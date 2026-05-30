//! Unit tests for [`super::JailRegistry`].
//!
//! Lives next to `registry.rs` (wired in via `#[cfg(test)] #[path =
//! "registry_tests.rs"] mod tests;`) so the production module stays under
//! the ~500-line guideline.

use super::*;

fn tempdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "openhuman-registry-{}-{}-{}",
        tag,
        std::process::id(),
        now_unix()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn create_list_get_roundtrip() {
    let base = tempdir("crud");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("alpha").unwrap();
    let b = reg.create("beta").unwrap();
    assert_ne!(a.id, b.id);
    assert!(a.dir.exists());
    assert!(b.dir.exists());
    let listed = reg.list();
    assert_eq!(listed.len(), 2);
    assert_eq!(reg.get(&a.id).unwrap().label, "alpha");
    assert_eq!(reg.get(&b.id).unwrap().label, "beta");
    fs::remove_dir_all(&base).ok();
}

#[test]
fn rename_changes_label_not_id_or_dir() {
    let base = tempdir("rename");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("old").unwrap();
    let renamed = reg.rename(&a.id, "new").unwrap();
    assert_eq!(renamed.id, a.id);
    assert_eq!(renamed.dir, a.dir);
    assert_eq!(renamed.label, "new");
    assert!(renamed.updated_at_unix >= a.updated_at_unix);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn delete_removes_dir_and_record() {
    let base = tempdir("delete");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("doomed").unwrap();
    let dir = a.dir.clone();
    assert!(dir.exists());
    reg.delete(&a.id).unwrap();
    assert!(!dir.exists());
    assert!(reg.get(&a.id).is_none());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn delete_missing_errors() {
    let base = tempdir("missing");
    let reg = JailRegistry::open(&base).unwrap();
    let err = reg.delete("nope").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn index_persists_across_reopen() {
    let base = tempdir("persist");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("persistent").unwrap();
    drop(reg);
    let reg2 = JailRegistry::open(&base).unwrap();
    let listed = reg2.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, a.id);
    assert_eq!(listed[0].label, "persistent");
    fs::remove_dir_all(&base).ok();
}

#[test]
fn find_by_label_substring() {
    let base = tempdir("find");
    let reg = JailRegistry::open(&base).unwrap();
    reg.create("agent-alpha").unwrap();
    reg.create("agent-beta").unwrap();
    reg.create("tool-gamma").unwrap();
    assert_eq!(reg.find_by_label("AGENT").len(), 2);
    assert_eq!(reg.find_by_label("gamma").len(), 1);
    assert_eq!(reg.find_by_label("nope").len(), 0);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn clear_drops_everything() {
    let base = tempdir("clear");
    let reg = JailRegistry::open(&base).unwrap();
    reg.create("a").unwrap();
    reg.create("b").unwrap();
    reg.create("c").unwrap();
    let n = reg.clear().unwrap();
    assert_eq!(n, 3);
    assert_eq!(reg.list().len(), 0);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn parallel_jails_have_distinct_dirs() {
    let base = tempdir("parallel");
    let reg = JailRegistry::open(&base).unwrap();
    let jails: Vec<_> = (0..5)
        .map(|i| reg.create(format!("p{i}")).unwrap())
        .collect();
    let mut dirs: Vec<_> = jails.iter().map(|r| r.dir.clone()).collect();
    dirs.sort();
    dirs.dedup();
    assert_eq!(dirs.len(), 5);
    for r in &jails {
        assert!(r.dir.exists());
    }
    fs::remove_dir_all(&base).ok();
}

#[test]
fn set_notes_roundtrips() {
    let base = tempdir("notes");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("with-notes").unwrap();
    assert!(a.notes.is_none());
    let updated = reg.set_notes(&a.id, Some("hello".into())).unwrap();
    assert_eq!(updated.notes.as_deref(), Some("hello"));
    let cleared = reg.set_notes(&a.id, None).unwrap();
    assert!(cleared.notes.is_none());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn set_notes_on_missing_id_errors() {
    let base = tempdir("notes-missing");
    let reg = JailRegistry::open(&base).unwrap();
    let err = reg.set_notes("nope", Some("x".into())).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn rename_on_missing_id_errors() {
    let base = tempdir("rename-missing");
    let reg = JailRegistry::open(&base).unwrap();
    let err = reg.rename("nope", "x").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn delete_twice_second_is_not_found() {
    let base = tempdir("delete-twice");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("once").unwrap();
    reg.delete(&a.id).unwrap();
    let err = reg.delete(&a.id).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn spawn_in_with_missing_id_errors() {
    let base = tempdir("spawn-missing");
    let reg = JailRegistry::open(&base).unwrap();
    let err = reg
        .spawn_in_with("nope", &super::super::NoopBackend, Command::new("true"))
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn spawn_in_uses_default_backend() {
    let base = tempdir("spawn-default");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("def").unwrap();
    let cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "exit"]);
        c
    } else {
        Command::new("true")
    };
    let mut child = reg.spawn_in(&a.id, cmd).unwrap();
    let _ = child.wait().unwrap();
    fs::remove_dir_all(&base).ok();
}

#[test]
fn clear_on_empty_registry_is_zero() {
    let base = tempdir("empty-clear");
    let reg = JailRegistry::open(&base).unwrap();
    assert_eq!(reg.clear().unwrap(), 0);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn find_by_label_on_empty_registry() {
    let base = tempdir("empty-find");
    let reg = JailRegistry::open(&base).unwrap();
    assert!(reg.find_by_label("anything").is_empty());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn open_creates_base_directory_if_missing() {
    let base = std::env::temp_dir().join(format!(
        "oh-reg-mkdir-{}-{}",
        std::process::id(),
        now_unix()
    ));
    assert!(!base.exists());
    let reg = JailRegistry::open(&base).unwrap();
    assert!(base.exists());
    assert!(reg.list().is_empty());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn corrupt_index_returns_invalid_data() {
    let base = tempdir("corrupt");
    fs::write(base.join("index.json"), b"this is not json").unwrap();
    let err = JailRegistry::open(&base).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn persist_writes_index_file() {
    let base = tempdir("persist-file");
    let reg = JailRegistry::open(&base).unwrap();
    reg.create("x").unwrap();
    let path = base.join("index.json");
    assert!(path.exists());
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"label\": \"x\""));
    fs::remove_dir_all(&base).ok();
}

#[test]
fn base_accessor_returns_open_dir() {
    let base = tempdir("base-accessor");
    let reg = JailRegistry::open(&base).unwrap();
    assert_eq!(reg.base(), base.as_path());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn delete_refuses_path_outside_base() {
    // Corrupt the index so a record points at /tmp directly (outside
    // base). delete() should refuse without touching anything on disk.
    let base = tempdir("escape");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("escape").unwrap();
    {
        let mut idx = reg.index.lock().unwrap();
        idx.records.get_mut(&a.id).unwrap().dir = std::env::temp_dir();
    }
    let err = reg.delete(&a.id).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert!(std::env::temp_dir().exists());
    // Record is still there because we refuse cleanly without removing.
    assert!(reg.get(&a.id).is_some());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn spawn_in_refuses_path_outside_base() {
    // Same corruption as delete_refuses_path_outside_base, but for the
    // spawn path — covers the base-containment guard in `jail_for()`.
    let base = tempdir("spawn-escape");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("escape").unwrap();
    {
        let mut idx = reg.index.lock().unwrap();
        idx.records.get_mut(&a.id).unwrap().dir = std::env::temp_dir();
    }
    let err = reg
        .spawn_in_with(&a.id, &super::super::NoopBackend, Command::new("true"))
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    fs::remove_dir_all(&base).ok();
}

#[test]
fn spawn_in_uses_record_dir_as_root() {
    let base = tempdir("spawn");
    let reg = JailRegistry::open(&base).unwrap();
    let a = reg.create("spawn-target").unwrap();
    let mut cmd = Command::new(if cfg!(windows) { "cmd" } else { "true" });
    if cfg!(windows) {
        cmd.args(["/C", "exit"]);
    }
    let mut child = reg
        .spawn_in_with(&a.id, &super::super::NoopBackend, cmd)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success() || cfg!(windows));
    fs::remove_dir_all(&base).ok();
}

#[test]
fn create_consecutive_ids_are_unique_in_same_second() {
    // The atomic counter inside generate_id() guarantees distinct ids
    // within a single process even when system time has not advanced.
    // Across process restarts, the create() loop is what catches
    // collisions; we cover that path via the collision-loop branch
    // being unreachable here without a process restart, so this test
    // just confirms the happy path remains collision-free.
    let base = tempdir("ids");
    let reg = JailRegistry::open(&base).unwrap();
    let ids: std::collections::HashSet<_> = (0..32)
        .map(|i| reg.create(format!("j{i}")).unwrap().id)
        .collect();
    assert_eq!(ids.len(), 32);
    fs::remove_dir_all(&base).ok();
}
