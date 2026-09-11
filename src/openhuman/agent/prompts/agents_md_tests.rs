use super::*;
use std::fs;

fn tmp() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!(
        "openhuman-agents-md-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn missing_file_returns_none() {
    let dir = tmp();
    assert_eq!(load_agents_md(&dir), None);
}

#[test]
fn present_file_returns_trimmed_content() {
    let dir = tmp();
    fs::write(dir.join(AGENTS_MD_FILENAME), "\n\n  hello world  \n\n").unwrap();
    assert_eq!(load_agents_md(&dir), Some("hello world".to_string()));
}

#[test]
fn empty_file_is_skipped() {
    let dir = tmp();
    fs::write(dir.join(AGENTS_MD_FILENAME), "   \n\t\n  ").unwrap();
    assert_eq!(load_agents_md(&dir), None);
}

#[test]
fn layers_load_both_when_dirs_differ() {
    let ws = tmp();
    let local = tmp();
    fs::write(ws.join(AGENTS_MD_FILENAME), "global rules").unwrap();
    fs::write(local.join(AGENTS_MD_FILENAME), "project rules").unwrap();
    let content = load_agents_md_layers(&ws, &local);
    assert_eq!(content.global.as_deref(), Some("global rules"));
    assert_eq!(content.local.as_deref(), Some("project rules"));
    assert!(!content.is_empty());
}

#[test]
fn same_dir_dedupes_to_global_only() {
    let ws = tmp();
    fs::write(ws.join(AGENTS_MD_FILENAME), "shared rules").unwrap();
    // Pass the same dir as both workspace and local.
    let content = load_agents_md_layers(&ws, &ws);
    assert_eq!(content.global.as_deref(), Some("shared rules"));
    assert_eq!(content.local, None, "same-dir local must dedupe to None");
}

#[test]
fn same_dir_dedupes_even_with_non_canonical_paths() {
    let ws = tmp();
    fs::write(ws.join(AGENTS_MD_FILENAME), "shared rules").unwrap();
    // A `./` prefixed variant must canonicalize to the same path.
    let dotted = ws.join(".").join("");
    let content = load_agents_md_layers(&ws, &dotted);
    assert_eq!(content.local, None, "canonicalized same-dir must dedupe");
}

#[test]
fn oversized_file_is_bounded_at_read_time() {
    let dir = tmp();
    // A file far larger than the read bound must not be slurped whole:
    // the loader returns bounded content (never the full body) so a
    // pathological AGENTS.md can't exhaust memory before rendering.
    let oversized = "a".repeat((MAX_AGENTS_MD_READ_BYTES as usize) * 3);
    fs::write(dir.join(AGENTS_MD_FILENAME), &oversized).unwrap();
    let loaded = load_agents_md(&dir).expect("non-empty file loads");
    assert!(
        (loaded.len() as u64) <= MAX_AGENTS_MD_READ_BYTES,
        "loader must bound the read to MAX_AGENTS_MD_READ_BYTES, got {} bytes",
        loaded.len()
    );
    assert!(
        loaded.len() < oversized.len(),
        "bounded content must be shorter than the on-disk file"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_agents_md_is_refused() {
    // A project-controlled AGENTS.md that symlinks to a secret outside the
    // action root must not be read into the prompt (path hardening).
    let dir = tmp();
    let secret_dir = tmp();
    let secret = secret_dir.join("secret.txt");
    fs::write(&secret, "TOP SECRET — must not leak").unwrap();
    std::os::unix::fs::symlink(&secret, dir.join(AGENTS_MD_FILENAME)).unwrap();
    assert_eq!(
        load_agents_md(&dir),
        None,
        "symlinked AGENTS.md must be refused"
    );
}

#[cfg(unix)]
#[test]
fn fifo_agents_md_is_refused_without_hanging() {
    use std::ffi::CString;
    // A FIFO (or device) is the kind of non-regular file a racing writer
    // could substitute after a naive stat check. The opened-fd `is_file`
    // fstat must reject it, and `O_NONBLOCK` must keep the open from
    // blocking on a FIFO that has no writer.
    let dir = tmp();
    let cpath = CString::new(dir.join(AGENTS_MD_FILENAME).to_str().unwrap()).unwrap();
    let rc = unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) };
    assert_eq!(rc, 0, "mkfifo failed: {}", std::io::Error::last_os_error());
    assert_eq!(
        load_agents_md(&dir),
        None,
        "FIFO AGENTS.md must be refused, not read"
    );
}

#[test]
fn both_missing_is_empty() {
    let ws = tmp();
    let local = tmp();
    let content = load_agents_md_layers(&ws, &local);
    assert!(content.is_empty());
}
