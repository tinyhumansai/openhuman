//! The archivist's per-turn episodic capture store: one markdown file per
//! turn, under the workspace's memory-tree content root.
//!
//! Layout, unchanged from the day the first turn was written:
//!
//! ```text
//! <workspace>/memory_tree/content/episodic/<session_id>/<seq:06>.md
//! ```
//!
//! Each file is a YAML front-matter block followed by the turn's prose body.
//! Sequence numbers are per-session and derived from the directory's contents
//! on every write, so the directory — not a counter held in memory or a row in
//! a database — is the source of truth for "which turn is this".
//!
//! # Why this is here and not in the engine (#5560)
//!
//! It was ported *out* of OpenHuman into `tinycortex` as
//! `memory::archivist::store` and is now back, for the same reason the
//! conversation store came back: nothing in the engine ever called it. Its only
//! caller has always been this hook — [`super::hook_impl`] writes both turns of
//! every completed turn, [`super::recap`] reads a session back — and its
//! imports are `std::fs`, `anyhow`, `serde_json` (as a string escaper, not as a
//! document format), `sha2` and `uuid`. No SQLite, no engine internals, no
//! `MemoryConfig` beyond one field. It was host code parked in a library.
//!
//! Bringing it home is what lets `tinycortex` leave this crate's
//! `[dependencies]`: the archivist runs on **every agent turn**, so as long as
//! its store lived behind that crate name, the engine was pinned into the
//! shipped binary by the hottest path in the harness.
//!
//! # Why it is not routed at `MemoryEpisodic` instead
//!
//! This is the obvious-looking migration and it is wrong, so it is worth
//! recording before someone re-derives it.
//!
//! The contract's episodic family (`insert_turn` / `session_turns`) is served
//! by `TinycortexProvider` out of `tinymemory_core::store::fts5` — the SQLite
//! full-text table. That is a **different store from this one**, not another
//! door onto it. [`super::hook_impl`] already calls both in the same function:
//! the FTS5 insert through the provider, and [`record_turn`] here. The pairing
//! is deliberate — it is the dual-write that lets the md store be validated
//! before the read side flips to it — and routing this half at the contract
//! would not move the write, it would delete it, taking every archived turn on
//! every user's disk with it.
//!
//! # What changed in the round trip, and what could not
//!
//! One signature: the engine's entry points took `&MemoryConfig` and read
//! exactly one field off it (`workspace`), so these take the workspace path
//! directly. The call sites previously built a `MemoryConfig::new(workspace)`
//! for the sole purpose of handing it back — the config carried a content-root
//! override and an embedding signature that this module has never read.
//!
//! Note that dropping it is what *preserves* behaviour rather than changing it.
//! [`content_root`] joins `memory_tree/content` onto the workspace
//! unconditionally; it deliberately does **not** consult
//! `Config::memory_tree_content_root`, which honours the
//! `memory_tree.content_root` override. A workspace that sets that override
//! still keeps its episodic turns under the workspace-rooted path, because
//! that is where they already are.
//!
//! Everything the on-disk bytes depend on is a verbatim copy: the `{:06}.md`
//! filename, the front-matter key order and escaping, the blank line between
//! front matter and body, the trailing-newline rule, the session-id sanitiser
//! (including its Unicode `is_alphanumeric`, its SHA-256 suffix, and the exact
//! six digest bytes it takes), and the create-new publish with its retry loop.
//! `store_tests` pins that claim against the engine's own implementation rather
//! than asserting it: it writes with one and reads with the other, in both
//! directions, and compares the raw file bytes.
//!
//! The one thing not carried over is the `Serialize`/`Deserialize` derive on
//! [`ArchivedTurn`]. The on-disk format is the hand-rolled front matter below,
//! not serde's rendering of this struct, and no caller serialises it — keeping
//! a derive that produces a *different* shape than the file it describes is an
//! invitation to write the wrong one.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Directory under the content root that holds per-session turn archives.
const EPISODIC_DIR: &str = "episodic";

/// One per-turn capture record, persisted by [`record_turn`] as a single md
/// file.
///
/// The field names mirror the contract's `EpisodicTurn` (and the legacy FTS5
/// `EpisodicEntry` before it) so the archivist's dual-write can hand the same
/// payload to both surfaces. They are not the same type: this one carries a
/// per-session `seq` assigned on write and epoch **milliseconds**, where the
/// contract carries a driver-assigned row id and epoch **seconds** as `f64`.
/// [`super::recap`] owns that conversion.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ArchivedTurn {
    /// Session this turn belongs to.
    pub(super) session_id: String,
    /// Per-session sequence number, assigned by [`record_turn`] on write.
    pub(super) seq: u32,
    /// Wall-clock timestamp of the turn (epoch milliseconds).
    pub(super) timestamp_ms: i64,
    /// `"user"` / `"assistant"` / `"system"` / `"tool"`.
    pub(super) role: String,
    /// Natural-language body.
    pub(super) content: String,
    /// Optional post-turn lesson (kept verbatim from the harness).
    pub(super) lesson: Option<String>,
    /// Serialized tool-call payload, when the turn issued any.
    pub(super) tool_calls_json: Option<String>,
    /// Cost in microdollars; 0 when not yet billed.
    pub(super) cost_microdollars: u64,
}

/// Content root for archivist episodic md files.
///
/// Workspace-rooted and nothing else — see the module docs for why this does
/// not go through `Config::memory_tree_content_root`.
fn content_root(workspace: &Path) -> PathBuf {
    workspace.join("memory_tree").join("content")
}

/// Directory holding one session's turns.
fn session_dir(workspace: &Path, session_id: &str) -> PathBuf {
    content_root(workspace)
        .join(EPISODIC_DIR)
        .join(sanitize_session(session_id))
}

/// Map any non-`[A-Za-z0-9_-]` character to `_` so a session id is always a
/// safe single path component.
fn sanitize_session(s: &str) -> String {
    sanitize_component_with_digest(s, |character| {
        character.is_alphanumeric() || character == '-' || character == '_'
    })
}

/// Sanitize one machine identifier into a path component while preserving
/// collision resistance. Safe, non-empty inputs retain their spelling;
/// transformed or empty inputs receive a short digest of the exact raw value.
///
/// The digest is what keeps `a/b` and `a?b` — which both sanitise to `a_b` —
/// in separate directories. Its width (six bytes of the SHA-256, hex) is part
/// of the derived path of every session whose id needed escaping, so it is not
/// a tunable.
fn sanitize_component_with_digest(raw: &str, allowed: impl Fn(char) -> bool) -> String {
    use sha2::{Digest, Sha256};

    let sanitized = raw
        .chars()
        .map(|character| if allowed(character) { character } else { '_' })
        .collect::<String>();
    if sanitized == raw && !sanitized.is_empty() {
        return sanitized;
    }
    let digest = Sha256::digest(raw.as_bytes());
    let suffix = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{sanitized}-{suffix}")
}

/// Next free sequence number for a session: one past the highest `NNNNNN.md`
/// already on disk (or 0 when the directory is empty/missing).
fn next_seq(dir: &Path) -> u32 {
    let mut max = -1i64;
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if let Some(stem) = s.strip_suffix(".md") {
                if let Ok(n) = stem.parse::<i64>() {
                    if n > max {
                        max = n;
                    }
                }
            }
        }
    }
    (max + 1) as u32
}

/// Render a turn as a YAML front-matter block followed by its body.
///
/// Key order, the two optional keys' omission when absent, the blank line
/// after the closing `---`, and the appended newline for a body that lacks one
/// are all load-bearing: they are the format every archived turn already on
/// disk was written in, and [`parse_turn`] reads back.
fn compose_turn(turn: &ArchivedTurn) -> String {
    let mut yaml = String::from("---\n");
    yaml.push_str(&format!("session_id: {}\n", yaml_escape(&turn.session_id)));
    yaml.push_str(&format!("seq: {}\n", turn.seq));
    yaml.push_str(&format!("timestamp_ms: {}\n", turn.timestamp_ms));
    yaml.push_str(&format!("role: {}\n", yaml_escape(&turn.role)));
    yaml.push_str(&format!("cost_microdollars: {}\n", turn.cost_microdollars));
    if let Some(lesson) = turn.lesson.as_ref() {
        yaml.push_str(&format!("lesson: {}\n", yaml_escape(lesson)));
    }
    if let Some(tc) = turn.tool_calls_json.as_ref() {
        yaml.push_str(&format!("tool_calls_json: {}\n", yaml_escape(tc)));
    }
    yaml.push_str("---\n\n");
    yaml.push_str(&turn.content);
    if !turn.content.ends_with('\n') {
        yaml.push('\n');
    }
    yaml
}

/// Encode a string as a JSON string literal. JSON quoted scalars are valid
/// YAML, and serde's encoder correctly escapes newlines and every control
/// character that must not appear literally in front matter.
fn yaml_escape(s: &str) -> String {
    serde_json::to_string(s).expect("serializing a string cannot fail")
}

/// Append a turn to its session's archive under `workspace`. Returns the
/// assigned `seq`.
///
/// `turn.seq` is ignored on input — the on-disk directory is the source of
/// truth and the returned [`ArchivedTurn`] carries the actually-assigned seq.
/// [`super::hook_impl`] threads that seq into the segment ops, which pair it
/// with the FTS5 episodic id, so the value returned here is the identity a
/// segment is later selected by.
pub(super) fn record_turn(workspace: &Path, mut turn: ArchivedTurn) -> Result<ArchivedTurn> {
    let dir = session_dir(workspace, &turn.session_id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to mkdir -p {}", dir.display()))?;
    loop {
        turn.seq = next_seq(&dir);
        let path = dir.join(format!("{:06}.md", turn.seq));
        let bytes = compose_turn(&turn).into_bytes();
        match write_if_new(&path, &bytes) {
            Ok(true) => return Ok(turn),
            // Another writer may have claimed the sequence after `next_seq`.
            // `write_if_new` uses create-new semantics, so retrying computes a
            // fresh sequence without ever overwriting the winning turn.
            Ok(false) => continue,
            Err(_) if path.exists() => continue,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to write episodic turn {}", path.display()))
            }
        }
    }
}

/// Read every turn for `session_id`, sorted by seq ascending. A missing session
/// directory yields an empty vec.
pub(super) fn session_entries(workspace: &Path, session_id: &str) -> Result<Vec<ArchivedTurn>> {
    let dir = session_dir(workspace, session_id);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<(u32, PathBuf)> = fs::read_dir(&dir)
        .with_context(|| format!("failed to read_dir {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            let stem = s.strip_suffix(".md")?;
            let seq = stem.parse::<u32>().ok()?;
            Some((seq, e.path()))
        })
        .collect();
    files.sort_by_key(|(seq, _)| *seq);
    let mut out = Vec::with_capacity(files.len());
    for (_, path) in files {
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        if let Some(turn) = parse_turn(&text) {
            out.push(turn);
        }
    }
    Ok(out)
}

/// Parse a front-matter + body md file back into an [`ArchivedTurn`].
///
/// Returns `None` for a file that is not front-matter shaped at all. A file
/// that *is* but carries an unparseable scalar keeps the field's default
/// rather than dropping the turn — a malformed cost must not lose the prose.
fn parse_turn(text: &str) -> Option<ArchivedTurn> {
    let body_start = text.strip_prefix("---\n")?;
    let end = body_start.find("\n---\n")?;
    let (yaml, rest) = body_start.split_at(end);
    let body = rest.strip_prefix("\n---\n").unwrap_or(rest).to_string();
    let mut turn = ArchivedTurn::default();
    for line in yaml.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        let v_unquoted = serde_json::from_str::<String>(v).unwrap_or_else(|_| v.to_string());
        match k {
            "session_id" => turn.session_id = v_unquoted,
            "seq" => turn.seq = v_unquoted.parse().unwrap_or(0),
            "timestamp_ms" => turn.timestamp_ms = v_unquoted.parse().unwrap_or(0),
            "role" => turn.role = v_unquoted,
            "cost_microdollars" => turn.cost_microdollars = v_unquoted.parse().unwrap_or(0),
            "lesson" => turn.lesson = Some(v_unquoted),
            "tool_calls_json" => turn.tool_calls_json = Some(v_unquoted),
            _ => {}
        }
    }
    // Strip the single blank line compose() writes between the closing `---\n`
    // and the body, then trim the trailing newline. Internal blank lines in the
    // body are preserved.
    turn.content = body
        .strip_prefix('\n')
        .unwrap_or(body.as_str())
        .trim_end()
        .to_string();
    Some(turn)
}

/// Write `bytes` atomically to `abs_path` if the file does not already exist.
///
/// Returns `Ok(true)` when newly written, `Ok(false)` when it already existed.
///
/// **Immutability contract**: once a file exists at `abs_path` it is never
/// overwritten. [`record_turn`]'s retry loop depends on that being enforced by
/// the filesystem rather than by the `exists()` check above it — two writers
/// can compute the same `next_seq` before either has published, and only a
/// create-new publish makes the loser observable as a loser.
fn write_if_new(abs_path: &Path, bytes: &[u8]) -> Result<bool> {
    use std::io::Write;

    if abs_path.exists() {
        return Ok(false);
    }

    let parent = abs_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| anyhow::anyhow!("create_dir_all {:?}: {e}", parent))?;

    let tmp_name = format!(".tmp_{}.md", uuid::Uuid::new_v4().simple());
    let tmp_path = parent.join(&tmp_name);

    {
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| anyhow::anyhow!("create tempfile {:?}: {e}", tmp_path))?;
        f.write_all(bytes)
            .map_err(|e| anyhow::anyhow!("write tempfile {:?}: {e}", tmp_path))?;
        f.sync_all()
            .map_err(|e| anyhow::anyhow!("fsync tempfile {:?}: {e}", tmp_path))?;
    }

    // Publish without replacement. A sibling hard link is atomic and fails
    // with AlreadyExists when another writer won the same destination; plain
    // `rename` cannot provide this contract because it replaces the target on
    // Unix.
    match std::fs::hard_link(&tmp_path, abs_path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp_path);
            // fsync the parent directory so the rename is durable across a crash.
            #[cfg(unix)]
            if let Some(parent) = abs_path.parent() {
                if let Ok(dir) = std::fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
            }
            Ok(true)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            if abs_path.exists() {
                // Lost the race — another writer created the file first.
                Ok(false)
            } else {
                Err(anyhow::anyhow!(
                    "filesystem does not support required atomic hard-link publish {:?} -> {:?}: {e}",
                    tmp_path,
                    abs_path
                ))
            }
        }
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
