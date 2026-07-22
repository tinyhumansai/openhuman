//! AGENTS.md instruction loading — OpenHuman's analog of Claude Code's
//! `CLAUDE.md` / Codex's `AGENTS.md`.
//!
//! Loads configurable instruction text from `AGENTS.md` files at two layers:
//!
//! 1. **Global** — `<workspace_dir>/AGENTS.md`: the user's OpenHuman workspace
//!    (where `SOUL.md` / `USER.md` already live). Applies to every run.
//! 2. **Local / project** — `<effective action_dir>/AGENTS.md`: the folder the
//!    agent is actually operating in. For sub-agent runs with a
//!    `worktree_action_dir` override, that override dir is the local layer.
//!
//! The loaded strings are threaded into
//! [`crate::openhuman::context::prompt::PromptContext`] and rendered by
//! `AgentsInstructionsSection` **once at system-prompt build time** — never
//! re-read per turn — so the frozen system-prompt prefix / KV-cache contract is
//! preserved. Missing, unreadable, or empty files are silently skipped. The
//! renderer caps each layer at
//! [`crate::openhuman::context::prompt::BOOTSTRAP_MAX_CHARS`] with a
//! `[... truncated]` marker.

use super::types::BOOTSTRAP_MAX_CHARS;
use std::io::Read;
use std::path::Path;

/// The instruction file name loaded at each layer.
pub const AGENTS_MD_FILENAME: &str = "AGENTS.md";

/// Hard cap on the number of bytes read from an `AGENTS.md` before the
/// renderer's per-layer character cap ([`BOOTSTRAP_MAX_CHARS`]) applies.
///
/// `AGENTS.md` is an untrusted, user/project-controlled file. Reading it whole
/// via `read_to_string` would allocate an arbitrarily large buffer — a
/// multi-megabyte file could stall prompt construction or exhaust memory
/// **before** the renderer ever gets a chance to truncate. Bounding the read
/// here fixes that at the root. At UTF-8's worst case of 4 bytes per character
/// this budget still yields at least `BOOTSTRAP_MAX_CHARS` characters (plus
/// slack), so the renderer's cap + `[... truncated]` marker still fire exactly
/// as before for any file large enough to matter — the read bound is invisible
/// to well-formed files and only clamps pathological ones.
const MAX_AGENTS_MD_READ_BYTES: u64 = (BOOTSTRAP_MAX_CHARS as u64) * 4 + 1024;

/// Pre-loaded `AGENTS.md` contents for the global + local layers.
///
/// Both fields are `None` when the corresponding file is absent / empty /
/// unreadable, or (for `local`) when it deduplicates against the global layer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentsMdContent {
    /// `<workspace_dir>/AGENTS.md` — the workspace-global layer.
    pub global: Option<String>,
    /// `<effective action_dir>/AGENTS.md` — the project layer. `None` when the
    /// local dir resolves to the same path as the workspace dir (the file is
    /// then loaded once, into [`Self::global`]).
    pub local: Option<String>,
}

impl AgentsMdContent {
    /// Whether either layer carries content.
    pub fn is_empty(&self) -> bool {
        self.global.is_none() && self.local.is_none()
    }
}

/// Load a single `AGENTS.md` from `dir`.
///
/// Returns `Some(trimmed_content)` when the file exists, is readable, and is
/// non-empty after trimming. Missing / unreadable / empty files yield `None`
/// (silently skipped) — callers inject the result unconditionally without a
/// noisy placeholder. Never logs the file contents, only paths and sizes.
pub fn load_agents_md(dir: &Path) -> Option<String> {
    let path = dir.join(AGENTS_MD_FILENAME);
    // Path hardening (race-free): the project-layer AGENTS.md lives in the
    // agent's user/project-controlled action dir, so a checkout could make it a
    // symlink pointing outside the root (a home-directory secret, a device node)
    // and have its bytes read into the system prompt and shipped to the
    // configured inference provider. We open with `O_NOFOLLOW` (Unix), so the
    // kernel atomically refuses to follow a final-component symlink at open
    // time — there is no stat-then-open window a racing writer could exploit by
    // swapping a checked regular file for a symlink. We then fstat the *opened*
    // handle and require a regular file (rejecting a directory / FIFO / socket /
    // device substituted in the same race), and — defence in depth against a
    // symlinked *parent* component — require the canonical path to stay under
    // `dir` (mirroring `validate_path_within_root`, used for agent-definition
    // TOML). All rejections skip silently, exactly like a missing file.
    let file = match open_no_follow(&path) {
        Ok(f) => f,
        Err(e) => {
            if is_symlink_refusal(&e) {
                log::warn!(
                    "[agents_md] refusing to read symlinked {} (path hardening)",
                    path.display()
                );
            } else if e.kind() == std::io::ErrorKind::NotFound {
                log::debug!("[agents_md] no AGENTS.md at {}", path.display());
            } else {
                log::debug!("[agents_md] failed to open {}: {e}", path.display());
            }
            return None;
        }
    };
    // fstat on the opened fd (race-free): reject anything that is not a regular
    // file — a directory, FIFO, socket, or device that slipped through.
    match file.metadata() {
        Ok(m) if m.is_file() => {}
        Ok(_) => {
            log::debug!(
                "[agents_md] {} is not a regular file; skipping",
                path.display()
            );
            return None;
        }
        Err(e) => {
            log::debug!("[agents_md] failed to stat opened {}: {e}", path.display());
            return None;
        }
    }
    if let Err(e) = crate::openhuman::security::validate_path_within_root(&path, dir) {
        log::warn!(
            "[agents_md] refusing to read {} outside its root: {e}",
            path.display()
        );
        return None;
    }
    // Bounded read: never slurp an arbitrarily large untrusted file whole into
    // memory. We `take(MAX_AGENTS_MD_READ_BYTES)` rather than `read_to_string`,
    // so a pathological multi-MB AGENTS.md can't stall prompt construction or
    // exhaust memory before the renderer's char cap applies.
    let mut buf = Vec::new();
    if let Err(e) = file.take(MAX_AGENTS_MD_READ_BYTES).read_to_end(&mut buf) {
        log::debug!("[agents_md] failed to read {}: {e}", path.display());
        return None;
    }
    let hit_read_bound = buf.len() as u64 >= MAX_AGENTS_MD_READ_BYTES;
    // `from_utf8_lossy` tolerates a multi-byte char clipped at the read bound by
    // substituting the replacement character for the trailing partial bytes.
    let content = String::from_utf8_lossy(&buf);
    let trimmed = content.trim();
    if trimmed.is_empty() {
        log::debug!("[agents_md] skipped empty {}", path.display());
        return None;
    }
    if hit_read_bound {
        log::debug!(
            "[agents_md] {} exceeds {} bytes; read bounded before render-time cap",
            path.display(),
            MAX_AGENTS_MD_READ_BYTES
        );
    }
    log::debug!(
        "[agents_md] loaded {} ({} chars)",
        path.display(),
        trimmed.chars().count()
    );
    Some(trimmed.to_string())
}

/// Load the global + local `AGENTS.md` layers given the workspace dir and the
/// effective local (action) dir.
///
/// Global renders first, local second (project instructions layered after).
/// **Dedupe:** when the two dirs resolve to the same path the file is loaded
/// once, into [`AgentsMdContent::global`], and `local` stays `None` so the same
/// instructions are never injected twice.
pub fn load_agents_md_layers(workspace_dir: &Path, local_dir: &Path) -> AgentsMdContent {
    let global = load_agents_md(workspace_dir);
    let same_dir = paths_equal(workspace_dir, local_dir);
    let local = if same_dir {
        log::debug!(
            "[agents_md] local dir {} == workspace dir; loaded once (deduped)",
            local_dir.display()
        );
        None
    } else {
        load_agents_md(local_dir)
    };
    AgentsMdContent { global, local }
}

/// Compare two directory paths for identity, canonicalizing when possible so
/// `./foo` and `foo` (or symlinked equivalents) dedupe. Falls back to literal
/// equality when canonicalization fails (e.g. a dir that does not exist yet).
fn paths_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Open `path` read-only while refusing to follow a final-component symlink.
///
/// On Unix this passes `O_NOFOLLOW` (the kernel returns `ELOOP`/`EMLINK` instead
/// of opening the symlink target) plus `O_NONBLOCK` (never block on a FIFO /
/// device a racing writer may substitute — it is rejected by the `is_file`
/// fstat check afterwards). This closes the check-to-open race that a
/// stat-then-`File::open` sequence would leave open. See [`load_agents_md`].
#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

/// Non-Unix fallback: best-effort pre-open symlink check. Windows symlink
/// creation requires elevation / developer mode, so the residual
/// check-to-open race is low risk on these platforms.
#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to follow symlink",
        ));
    }
    std::fs::File::open(path)
}

/// Whether an [`open_no_follow`] error is the "refused a symlink" signal (as
/// opposed to a genuine I/O failure), so the caller can log it distinctly.
#[cfg(unix)]
fn is_symlink_refusal(e: &std::io::Error) -> bool {
    // `O_NOFOLLOW` on a symlink yields `ELOOP` on Linux/macOS and `EMLINK` on
    // some BSDs — either way the open was refused *because* it was a symlink.
    matches!(e.raw_os_error(), Some(v) if v == libc::ELOOP || v == libc::EMLINK)
}

#[cfg(not(unix))]
fn is_symlink_refusal(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::InvalidInput
}

#[cfg(test)]
mod tests {
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
}
