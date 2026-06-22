//! Unified-diff compressor.
//!
//! Clean-room port of headroom's `DiffCompressor` (Apache-2.0), in the
//! lossy-but-bounded style of [`super::search`] / [`super::logs`] (the caller
//! offloads the original to CCR for retrieval):
//!
//! - **Always keep** structural lines: `diff --git`, `index`, `---`/`+++`
//!   file headers, and `@@` hunk headers.
//! - **Always keep** changed lines (`+`/`-`) — they are the signal.
//! - **Collapse** long runs of unchanged context (lines starting with a
//!   space) down to a few anchor lines plus a `[... N context lines ...]`
//!   marker, so the model still sees where a change sits without paying for
//!   the whole untouched neighbourhood.
//! - **Summarize** high-volume / low-value files (lockfiles, minified
//!   bundles): the hunk body collapses to a one-line `+A/-B` summary.
//!
//! Changed lines are never dropped, so the diff stays faithful to *what
//! changed* even when the surrounding context is trimmed.

use super::Compacted;
use std::fmt::Write as _;

/// Context lines kept on each side of a changed run before collapsing.
pub const CONTEXT_ANCHOR: usize = 3;
/// A run of unchanged context longer than this collapses to a marker.
pub const CONTEXT_COLLAPSE_THRESHOLD: usize = 8;

/// Compress a unified diff. Returns `None` when there's nothing structural to
/// work with or compression wouldn't shrink it. Lossy when it fires (collapses
/// context / summarizes hunks); the caller offloads the original to CCR.
pub fn compress(content: &str) -> Option<Compacted> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let mut out = String::with_capacity(content.len() / 2 + 64);
    let mut i = 0usize;
    let mut current_file_is_noisy = false;
    let mut saw_hunk = false;

    while i < lines.len() {
        let line = lines[i];

        // File header — note whether this file is a lockfile/bundle we summarize.
        if line.starts_with("diff --git ") {
            current_file_is_noisy = is_noisy_path(line);
            let _ = writeln!(out, "{line}");
            i += 1;
            continue;
        }
        if is_structural(line) {
            saw_hunk |= line.starts_with("@@");
            // For a noisy file, collapse the entire hunk body to a summary.
            if current_file_is_noisy && line.starts_with("@@") {
                let _ = writeln!(out, "{line}");
                i += 1;
                let (added, removed, consumed) = summarize_hunk_body(&lines[i..]);
                let _ = writeln!(
                    out,
                    "[... lockfile/bundle hunk: +{added}/-{removed} line(s) omitted ...]"
                );
                i += consumed;
                continue;
            }
            let _ = writeln!(out, "{line}");
            i += 1;
            continue;
        }

        // Context line — collapse long unchanged runs.
        if is_context(line) {
            let start = i;
            while i < lines.len() && is_context(lines[i]) {
                i += 1;
            }
            let run = &lines[start..i];
            if run.len() > CONTEXT_COLLAPSE_THRESHOLD {
                for l in &run[..CONTEXT_ANCHOR] {
                    let _ = writeln!(out, "{l}");
                }
                let omitted = run.len() - 2 * CONTEXT_ANCHOR;
                let _ = writeln!(out, "[... {omitted} context line(s) omitted ...]");
                for l in &run[run.len() - CONTEXT_ANCHOR..] {
                    let _ = writeln!(out, "{l}");
                }
            } else {
                for l in run {
                    let _ = writeln!(out, "{l}");
                }
            }
            continue;
        }

        // Changed line (+/-) or anything else — keep verbatim.
        let _ = writeln!(out, "{line}");
        i += 1;
    }

    if !saw_hunk {
        return None;
    }
    if out.len() >= content.len() {
        return None;
    }
    log::debug!(
        "[compaction][diff] {} -> {} bytes ({} input lines)",
        content.len(),
        out.len(),
        lines.len(),
    );
    Some(Compacted::lossy(out.trim_end().to_string()))
}

/// Structural diff lines that must always survive.
fn is_structural(line: &str) -> bool {
    line.starts_with("@@")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("index ")
        || line.starts_with("new file")
        || line.starts_with("deleted file")
        || line.starts_with("rename ")
        || line.starts_with("similarity ")
        || line.starts_with("Binary files")
}

/// A unified-diff context line: leading space, not a `+`/`-` change and not a
/// `---`/`+++` header (those are structural and handled first).
fn is_context(line: &str) -> bool {
    line.starts_with(' ')
}

/// Count added/removed lines in a hunk body until the next hunk/file header,
/// returning how many lines were consumed.
fn summarize_hunk_body(lines: &[&str]) -> (usize, usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut n = 0usize;
    for &line in lines {
        if line.starts_with("@@") || line.starts_with("diff --git ") {
            break;
        }
        if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
        n += 1;
    }
    (added, removed, n)
}

/// Lockfiles and generated bundles whose diff body is rarely worth reading in
/// full. Matched against the `diff --git a/<path> b/<path>` header.
fn is_noisy_path(diff_git_line: &str) -> bool {
    let l = diff_git_line.to_ascii_lowercase();
    const NOISY: &[&str] = &[
        "cargo.lock",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "composer.lock",
        "poetry.lock",
        "gemfile.lock",
        ".min.js",
        ".min.css",
        ".map",
        "go.sum",
    ];
    NOISY.iter().any(|p| l.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_changed_lines_collapses_context() {
        let mut s = String::from("diff --git a/x.rs b/x.rs\n@@ -1,40 +1,41 @@\n");
        for i in 0..20 {
            let _ = writeln!(s, " context line {i} unchanged here");
        }
        let _ = writeln!(s, "-old changed line");
        let _ = writeln!(s, "+new changed line");
        for i in 0..20 {
            let _ = writeln!(s, " more context {i} unchanged");
        }
        let out = compress(&s).expect("compresses").text;
        assert!(out.contains("-old changed line"), "{out}");
        assert!(out.contains("+new changed line"), "{out}");
        assert!(out.contains("context line(s) omitted"), "{out}");
        assert!(out.contains("@@ -1,40 +1,41 @@"));
        assert!(out.len() < s.len());
    }

    #[test]
    fn summarizes_lockfile_hunk() {
        let mut s = String::from("diff --git a/Cargo.lock b/Cargo.lock\n@@ -1,60 +1,80 @@\n");
        for i in 0..40 {
            let _ = writeln!(s, "+ new dep entry {i}");
        }
        for i in 0..20 {
            let _ = writeln!(s, "- old dep entry {i}");
        }
        let out = compress(&s).expect("compresses").text;
        assert!(out.contains("lockfile/bundle hunk"), "{out}");
        assert!(out.contains("Cargo.lock"));
        // Individual dep lines are gone.
        assert!(!out.contains("new dep entry 7"), "{out}");
        assert!(out.len() < s.len());
    }

    #[test]
    fn non_diff_returns_none() {
        assert!(compress("just some text\nno hunks here").is_none());
    }
}
