//! Phase-out migration: removes legacy `### PROFILE.md` blocks from
//! persisted session transcripts.
//!
//! `PROFILE.md` is being retired as a system-prompt injection. Earlier
//! builds wrote the file into the system prompt of every session and
//! froze those bytes inside `session_raw/{stem}.jsonl` (see
//! [`crate::openhuman::agent::harness::session::transcript`]). Without
//! this migration, an upgrade leaves the leaked content replaying
//! inside every resumed thread forever — the runtime loader reuses the
//! persisted system message verbatim for KV-cache stability.
//!
//! ## What it does
//!
//! 1. **Deletes `{workspace}/PROFILE.md`** if it exists. With the file
//!    gone, the renderer's `inject_workspace_file_capped` short-circuits
//!    silently — agents that still have `omit_profile = false`
//!    (orchestrator, welcome, trigger_*, help) produce a clean prompt
//!    on every new session without any source-code change.
//! 2. **Walks every JSONL transcript** under `{workspace}/session_raw/`
//!    (both the current flat layout and the legacy `DDMMYYYY/`
//!    date-grouped subdirs), reads each file, and — if the first
//!    message is a system message containing a `### PROFILE.md`
//!    heading — removes the heading and everything beneath it up to
//!    the next `### ` heading (or end-of-prompt). The transcript is
//!    rewritten in place via the same [`transcript::write_transcript`]
//!    used at runtime so the on-disk shape stays byte-compatible.
//! 3. **Sweeps `.md` companions** under `{workspace}/sessions/**/*.md`,
//!    rewriting in place any file whose body still contains a
//!    `### PROFILE.md` heading. The rest of the transcript
//!    (conversation messages, metadata header) is preserved — only
//!    the PROFILE.md block is removed. Supports both the new
//!    JSONL-companion format (`## [role]` headers, `---` separators)
//!    and the legacy HTML-comment format (`<!--MSG …-->` markers) via
//!    the boundary set in [`strip_profile_md_block`].
//!
//! ## Idempotency
//!
//! Gated externally by [`Config::schema_version`] — once the migration
//! succeeds and the bump is persisted, future launches skip it.
//! Internally self-idempotent too: a transcript without any
//! `### PROFILE.md` block is detected and left unchanged.
//!
//! ## Fresh installs
//!
//! When `session_raw/` does not exist or contains no transcripts the
//! migration short-circuits without scanning. The caller still bumps
//! `schema_version` so future launches don't re-check.

use crate::openhuman::agent::harness::session::transcript::{self, SessionTranscript};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Line that opens a `PROFILE.md` block in a system prompt. Matches the
/// output of `inject_workspace_file_capped` in `agent/prompts/mod.rs`.
const PROFILE_HEADING: &str = "### PROFILE.md";

/// Summary of what the migration touched in one workspace.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhaseOutStats {
    pub scanned: usize,
    pub cleaned: usize,
    pub skipped: usize,
    pub errors: usize,
    /// True when the legacy `PROFILE.md` file at the workspace root was
    /// deleted by this run. `false` when no file was present.
    pub profile_md_removed: bool,
    /// Number of stale `.md` companions / legacy-format transcripts
    /// under `sessions/**/*.md` that contained a `### PROFILE.md` block
    /// and were rewritten in place with the block stripped (proofs
    /// preserved).
    pub md_companions_altered: usize,
}

/// Run the migration over `workspace_dir`. Returns aggregate stats.
///
/// Per-file failures are logged and counted in `stats.errors` but do
/// not abort the walk — a single unreadable transcript shouldn't keep
/// the rest of the fleet stuck on the old data.
pub fn run(workspace_dir: &Path) -> Result<PhaseOutStats> {
    let mut stats = PhaseOutStats::default();

    // Final piece of the consumer-side phase-out: delete the legacy
    // `PROFILE.md` file at the workspace root. With the file gone the
    // renderer's `inject_workspace_file_capped` short-circuits silently,
    // so even the agents whose `omit_profile = false` still want it will
    // produce a clean prompt. This is intentionally done *before* the
    // transcript walk so that a partial run still removes the file.
    stats.profile_md_removed = remove_workspace_profile_md(workspace_dir, &mut stats);

    let raw_root = workspace_dir.join("session_raw");
    let transcripts = collect_jsonl_transcripts(&raw_root);
    if transcripts.is_empty() {
        log::info!(
            "[migration:phase-out-profile-md] no transcripts under {} — \
             continuing to .md sweep",
            raw_root.display()
        );
    }

    for path in transcripts {
        stats.scanned += 1;
        match process_transcript(&path) {
            Ok(true) => {
                stats.cleaned += 1;
                log::info!(
                    "[migration:phase-out-profile-md] cleaned path={}",
                    path.display()
                );
            }
            Ok(false) => {
                stats.skipped += 1;
                log::debug!(
                    "[migration:phase-out-profile-md] skipped (already clean) path={}",
                    path.display()
                );
            }
            Err(err) => {
                stats.errors += 1;
                log::warn!(
                    "[migration:phase-out-profile-md] failed path={}: {err:#}",
                    path.display()
                );
            }
        }
    }

    // Sweep stale `.md` companions and legacy-format transcripts that
    // still carry a `### PROFILE.md` block. The JSONL above is the
    // source of truth and was rewritten in-place where needed; these
    // `.md` files are either derived views (regenerated on next
    // session write) or legacy artifacts. Deletion is safe and lines
    // up with the phase-out framing.
    stats.md_companions_altered = sweep_tainted_md_companions(workspace_dir, &mut stats);

    log::info!(
        "[migration:phase-out-profile-md] complete scanned={} cleaned={} skipped={} errors={} \
         profile_md_removed={} md_companions_altered={}",
        stats.scanned,
        stats.cleaned,
        stats.skipped,
        stats.errors,
        stats.profile_md_removed,
        stats.md_companions_altered
    );

    Ok(stats)
}

/// Walk `{workspace}/sessions/**/*.md` and rewrite any file whose body
/// still contains a `### PROFILE.md` heading, stripping the block in
/// place. Returns the number of files mutated. Preserves the rest of
/// each transcript — conversations and proofs stay intact, only the
/// PROFILE.md section is removed.
///
/// **Layout assumption (one level deep).** The walk descends exactly one
/// level — into `sessions/<date>/*.md` — matching the layout produced
/// by [`crate::openhuman::agent::harness::session::transcript`]
/// (`sessions/YYYY_MM_DD/{stem}.md` for the new format, legacy
/// `sessions/DDMMYYYY/{stem}.md` for the date-grouped fallback). The
/// JSONL walk in [`collect_jsonl_transcripts`] makes the same
/// assumption. If a future change starts producing deeper nesting
/// (e.g. `sessions/YYYY/MM/DD/`), both walkers will silently miss the
/// inner files and this comment needs to be revisited.
///
/// Read/IO errors are folded into `stats.errors` and the walk continues.
fn sweep_tainted_md_companions(workspace_dir: &Path, stats: &mut PhaseOutStats) -> usize {
    let sessions_root = workspace_dir.join("sessions");
    if !sessions_root.is_dir() {
        return 0;
    }

    let mut altered = 0usize;
    let Ok(entries) = fs::read_dir(&sessions_root) else {
        return 0;
    };
    for entry in entries.flatten() {
        let sub = entry.path();
        if !sub.is_dir() {
            continue;
        }
        for md_path in md_files_in_dir(&sub) {
            match fs::read_to_string(&md_path) {
                Ok(body) => {
                    let Some(cleaned) = strip_profile_md_block(&body) else {
                        log::debug!(
                            "[migration:phase-out-profile-md] md clean, leaving path={}",
                            md_path.display()
                        );
                        continue;
                    };
                    match fs::write(&md_path, cleaned.as_bytes()) {
                        Ok(()) => {
                            altered += 1;
                            log::info!(
                                "[migration:phase-out-profile-md] altered tainted md path={}",
                                md_path.display()
                            );
                        }
                        Err(err) => {
                            stats.errors += 1;
                            log::warn!(
                                "[migration:phase-out-profile-md] failed to rewrite tainted md {}: {err:#}",
                                md_path.display()
                            );
                        }
                    }
                }
                Err(err) => {
                    stats.errors += 1;
                    log::warn!(
                        "[migration:phase-out-profile-md] failed to read md {}: {err:#}",
                        md_path.display()
                    );
                }
            }
        }
    }
    altered
}

fn md_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(p);
        }
    }
    out
}

/// Delete `{workspace_dir}/PROFILE.md` if it exists. Errors are folded
/// into `stats.errors`. Returns `true` when a file was removed.
fn remove_workspace_profile_md(workspace_dir: &Path, stats: &mut PhaseOutStats) -> bool {
    let path = workspace_dir.join("PROFILE.md");
    if !path.exists() {
        return false;
    }
    match fs::remove_file(&path) {
        Ok(()) => {
            log::info!(
                "[migration:phase-out-profile-md] removed legacy file path={}",
                path.display()
            );
            true
        }
        Err(err) => {
            stats.errors += 1;
            log::warn!(
                "[migration:phase-out-profile-md] failed to remove {}: {err:#}",
                path.display()
            );
            false
        }
    }
}

/// Process one transcript file. Returns `true` when the file was
/// rewritten with a cleaned system prompt.
fn process_transcript(path: &Path) -> Result<bool> {
    let mut session = transcript::read_transcript(path)
        .with_context(|| format!("read transcript {}", path.display()))?;

    if !strip_profile_md_in_first_system_message(&mut session) {
        return Ok(false);
    }

    transcript::write_transcript(path, &session.messages, &session.meta, None)
        .with_context(|| format!("rewrite cleaned transcript {}", path.display()))?;

    Ok(true)
}

/// Strip a `### PROFILE.md` block from the first system message of a
/// transcript. Returns `true` when the message was mutated.
pub(super) fn strip_profile_md_in_first_system_message(session: &mut SessionTranscript) -> bool {
    let Some(first) = session.messages.first_mut() else {
        return false;
    };
    if first.role != "system" {
        return false;
    }
    let Some(cleaned) = strip_profile_md_block(&first.content) else {
        return false;
    };
    first.content = cleaned;
    true
}

/// Strip a `### PROFILE.md` block from a string.
///
/// The block is anchored on a line equal to [`PROFILE_HEADING`] and
/// runs until the next *boundary* line or end-of-string. A boundary
/// line is any of:
///
/// - `### …` — another level-3 heading (the system-prompt-builder's
///   neighbouring sections).
/// - `---` — markdown horizontal rule. Never emitted inside a JSONL
///   system prompt by the renderer; **is** the message separator in
///   the new `.md` companion format ([`render_markdown`]).
/// - `## [` — the next-message header in a `.md` companion.
/// - `<!--/MSG-->` — the close tag in the legacy HTML-comment `.md`
///   format read by [`transcript::read_transcript_legacy_md`].
///
/// The extra boundaries make the same function safe to apply to either
/// a raw system-prompt string or a `.md` transcript body, without
/// special-casing per caller.
///
/// Trailing blank lines preceding the cut and leading blank lines
/// following it are absorbed so we don't leave a runaway gap.
///
/// Returns `Some(cleaned)` when the input was mutated, otherwise `None`.
pub(super) fn strip_profile_md_block(prompt: &str) -> Option<String> {
    let lines: Vec<&str> = prompt.split('\n').collect();

    let start = lines
        .iter()
        .position(|line| line.trim_end() == PROFILE_HEADING)?;

    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(i, line)| is_block_boundary(line).then_some(i))
        .unwrap_or(lines.len());

    let mut head_end = start;
    while head_end > 0 && lines[head_end - 1].trim().is_empty() {
        head_end -= 1;
    }
    let mut tail_start = end;
    while tail_start < lines.len() && lines[tail_start].trim().is_empty() {
        tail_start += 1;
    }

    let head = lines[..head_end].join("\n");
    let tail = lines[tail_start..].join("\n");

    let mut out = head;
    if !tail.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&tail);
    }
    // Defensive: a `### PROFILE.md` heading was located but the
    // reconstructed string is byte-identical to the input (degenerate
    // whitespace shapes can land here). Treat that as "no change" so
    // callers don't churn the file uselessly or under-count skips.
    if out == prompt {
        return None;
    }
    Some(out)
}

/// True when `line` ends the `### PROFILE.md` block — see
/// [`strip_profile_md_block`] for the rationale behind each marker.
fn is_block_boundary(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.starts_with("### ")
        || trimmed == "---"
        || trimmed.starts_with("## [")
        || trimmed == "<!--/MSG-->"
}

/// Collect every `*.jsonl` under `session_raw/` (flat) and its legacy
/// `DDMMYYYY/` subdirectories. Returns empty when the root does not
/// exist. Directory-iteration errors are swallowed — the migration
/// should never abort startup on a single unreadable directory.
fn collect_jsonl_transcripts(raw_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if !raw_root.is_dir() {
        return paths;
    }
    push_jsonl_in_dir(raw_root, &mut paths);
    if let Ok(entries) = fs::read_dir(raw_root) {
        for entry in entries.flatten() {
            let sub = entry.path();
            if sub.is_dir() {
                push_jsonl_in_dir(&sub, &mut paths);
            }
        }
    }
    paths.sort();
    paths
}

fn push_jsonl_in_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

#[cfg(test)]
#[path = "phase_out_profile_md_tests.rs"]
mod tests;
