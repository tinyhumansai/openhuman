use super::*;

#[test]
fn defaults_are_permissive() {
    let j = Jail::new("/tmp", "x");
    assert!(j.allow_net);
    assert!(j.allow_subprocess);
    assert_eq!(j.label, "x");
    assert!(j.read_only.is_empty());
}

#[test]
fn deny_net_is_idempotent() {
    let j = Jail::new("/tmp", "x").deny_net().deny_net();
    assert!(!j.allow_net);
}

#[test]
fn deny_subprocess_is_idempotent() {
    let j = Jail::new("/tmp", "x").deny_subprocess().deny_subprocess();
    assert!(!j.allow_subprocess);
}

#[test]
fn add_read_only_appends_in_order() {
    let j = Jail::new("/tmp", "x")
        .add_read_only("/a")
        .add_read_only("/b")
        .add_read_only("/c");
    assert_eq!(j.read_only.len(), 3);
    assert_eq!(j.read_only[0], PathBuf::from("/a"));
    assert_eq!(j.read_only[2], PathBuf::from("/c"));
}

#[test]
fn canonicalize_resolves_real_path() {
    let dir = std::env::temp_dir();
    let mut j = Jail::new(&dir, "x");
    j.canonicalize().unwrap();
    // After canonicalize, root has no `..` and resolves to a real path.
    assert!(j.root.is_absolute());
    assert!(j.root.exists());
}

#[test]
fn canonicalize_swallows_missing_read_only() {
    // read_only entries that don't exist are silently dropped from
    // canonicalization (they stay as-is). Verify no panic.
    let dir = std::env::temp_dir();
    let mut j = Jail::new(&dir, "x").add_read_only("/this/never/existed");
    j.canonicalize().unwrap();
    assert_eq!(j.read_only.len(), 1);
}

#[test]
fn canonicalize_errors_on_missing_root() {
    let mut j = Jail::new("/no/such/root/here", "x");
    let err = j.canonicalize().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}
