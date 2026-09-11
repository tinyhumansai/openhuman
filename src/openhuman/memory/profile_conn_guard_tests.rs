//! The `profile_conn` confinement guard.
//!
//! `MemoryClient::profile_conn()` hands out a raw `Arc<Mutex<Connection>>` that
//! no decorator can wrap, so no caller outside the memory family may hold one.
//! Before the extraction it was `pub(in crate)` and the compiler enforced that.
//! The family now spans `tinymemory-core` and this module, so the rule has to
//! be a test — and a test that *names the offending file*, because a visibility
//! error at a call site reads as "private method", not as "you are reaching
//! around the guard".

/// /// Before the typed-store change this reported
/// `agent/learning/{schemas,startup,tools}.rs` (six call sites).
#[test]
fn profile_conn_is_confined_to_the_memory_family() {
    fn rs_files_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rs_files_under(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let family = root.join("openhuman").join("memory");
    let mut files = Vec::new();
    rs_files_under(&root, &mut files);

    let mut outside = Vec::new();
    for path in files {
        if path.starts_with(&family) {
            continue;
        }
        // By-path test files are out of scope, matching the sibling lints
        // (`direct_engine_refs`, `bypass_allowlist`): a `_tests.rs` file that
        // writes through the provider and then proves the row landed with a
        // raw read is using the connection as an oracle, not as a door. The
        // production rule is unchanged — no non-test file outside the family
        // may touch it.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_tests.rs") || n == "tests.rs")
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains(".profile_conn(") {
                outside.push(path.display().to_string());
                break;
            }
        }
    }
    assert!(
        outside.is_empty(),
        "raw profile connections reached from outside the memory family: {outside:?}\n\
         Use `MemoryClient::profile_store()`; every SQL statement against \
         user_profile belongs inside the memory family."
    );
}
