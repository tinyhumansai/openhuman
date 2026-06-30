//! Unified-diff compressor.
//!
//! Clean-room port of Headroom's `DiffCompressor` (Apache-2.0). Keeps the
//! signal (changed lines, structural headers, hunk headers) and collapses long
//! runs of unchanged context to an anchor + marker. Lockfile/bundle hunks
//! collapse to a one-line `+A/-B` summary. The router offloads the original to
//! CCR, so the dropped context stays recoverable.

use async_trait::async_trait;
use std::fmt::Write as _;

use super::Compressor;
use crate::openhuman::tokenjuice::text::ansi::strip_ansi;
use crate::openhuman::tokenjuice::types::{
    CompressInput, CompressOptions, CompressOutput, CompressorKind,
};

/// Context lines kept on each side of a changed run before collapsing.
pub const CONTEXT_ANCHOR: usize = 3;
/// A run of unchanged context longer than this collapses to a marker.
pub const CONTEXT_COLLAPSE_THRESHOLD: usize = 8;

pub struct DiffCompressor;

#[async_trait]
impl Compressor for DiffCompressor {
    fn kind(&self) -> CompressorKind {
        CompressorKind::Diff
    }

    async fn compress(
        &self,
        input: &CompressInput<'_>,
        _opts: &CompressOptions,
    ) -> Option<CompressOutput> {
        compress(input.content)
    }
}

/// Compress a unified diff. Returns `None` when there's nothing structural to
/// work with or compression wouldn't shrink it.
pub fn compress(content: &str) -> Option<CompressOutput> {
    let stripped = strip_ansi(content);
    let lines: Vec<&str> = stripped.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let mut out = String::with_capacity(stripped.len() / 2 + 64);
    let mut i = 0usize;
    let mut current_file_is_noisy = false;
    let mut saw_hunk = false;

    while i < lines.len() {
        let line = lines[i];

        if line.starts_with("diff --git ") {
            current_file_is_noisy = is_noisy_path(line);
            let _ = writeln!(out, "{line}");
            i += 1;
            continue;
        }
        if is_structural(line) {
            saw_hunk |= line.starts_with("@@");
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
        "[tokenjuice][diff] {} -> {} bytes ({} input lines)",
        content.len(),
        out.len(),
        lines.len(),
    );
    Some(CompressOutput::lossy(
        out.trim_end().to_string(),
        CompressorKind::Diff,
    ))
}

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

fn is_context(line: &str) -> bool {
    line.starts_with(' ')
}

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
        assert!(!out.contains("new dep entry 7"), "{out}");
        assert!(out.len() < s.len());
    }

    #[test]
    fn non_diff_returns_none() {
        assert!(compress("just some text\nno hunks here").is_none());
    }
}
