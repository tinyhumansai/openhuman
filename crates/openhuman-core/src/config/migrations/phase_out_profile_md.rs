//! Phase-out migration: retires workspace `PROFILE.md` without modifying
//! legacy transcript source files.
//!
//! `PROFILE.md` is no longer injected into new prompts. Older transcripts may
//! still contain its section; the session importer removes that section from
//! imported bytes in memory while preserving all source files.
//!
//! ## What it does
//!
//! 1. Leaves `PROFILE.md`, JSONL transcripts, and Markdown companions
//!    byte-for-byte intact.
//! 2. `storage::import::sessions` strips the old prompt section from imported
//!    JSONL bytes through TinyAgents' format-aware pure transformer.
//!
//! ## Idempotency
//!
//! Gated externally by [`Config::schema_version`] — once the migration
//! succeeds and the bump is persisted, future launches skip it.
//! The import-time byte transform is idempotent.
//!
//! ## Fresh installs
//!
//! When `session_raw/` does not exist or contains no transcripts the
//! migration short-circuits without scanning. The caller still bumps
//! `schema_version` so future launches don't re-check.

use anyhow::Result;
use std::path::Path;

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
    /// Retained for report compatibility. Source profile files are preserved,
    /// so this is always false.
    pub profile_md_removed: bool,
    /// Retained for migration report compatibility. Source companions are
    /// never rewritten, so this is always zero.
    pub md_companions_altered: usize,
}

/// Run the migration over `workspace_dir`. Returns aggregate stats.
///
/// Legacy transcript files are handled by the read-only session importer.
pub fn run(_workspace_dir: &Path) -> Result<PhaseOutStats> {
    let stats = PhaseOutStats::default();

    // Transcript JSONL and Markdown companions are immutable source
    // evidence. The session importer applies the pure TinyAgents rewrite to
    // transcript bytes in memory before importing them into the destination.

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
///   format read by `tinyagents_session::transcript::read_transcript_legacy_md`.
///
/// The extra boundaries make the same function safe to apply to either
/// a raw system-prompt string or a `.md` transcript body, without
/// special-casing per caller.
///
/// Trailing blank lines preceding the cut and leading blank lines
/// following it are absorbed so we don't leave a runaway gap.
///
/// Returns `Some(cleaned)` when the input was mutated, otherwise `None`.
pub(crate) fn strip_profile_md_block(prompt: &str) -> Option<String> {
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

#[cfg(test)]
#[path = "phase_out_profile_md_tests.rs"]
mod tests;
