//! Build/test/lint log compressor.
//!
//! Clean-room port of headroom's `LogCompressor` (Apache-2.0). Keeps the
//! lines an agent acts on and drops the ceremony:
//!
//! - **errors** — keep up to [`MAX_ERRORS`] (biased to the first and last,
//!   which are usually the root cause and the abort line).
//! - **warnings** — keep up to [`MAX_WARNINGS`], de-duplicated (the "same
//!   deprecation × 47" case collapses to one).
//! - **stack traces** — keep up to [`MAX_STACK_TRACES`] runs of indented
//!   frames, each capped at [`STACK_TRACE_MAX_LINES`].
//! - **summary lines** — always keep test/build summaries
//!   (`test result: ...`, `N passed; M failed`, `error: aborting`).
//! - hard cap of [`MAX_TOTAL_LINES`] kept lines overall.
//!
//! Output preserves original line order and notes how many lines were
//! dropped. Lossy-but-bounded: first/last errors and the summary always
//! survive.

use super::signals::{severity, Severity};
use super::Compacted;
use std::collections::HashSet;
use std::fmt::Write as _;

pub const MAX_ERRORS: usize = 10;
pub const MAX_WARNINGS: usize = 5;
pub const MAX_STACK_TRACES: usize = 3;
pub const STACK_TRACE_MAX_LINES: usize = 20;
pub const MAX_TOTAL_LINES: usize = 100;

/// Compress a build/test/lint log. Returns `None` when nothing can be saved
/// (few lines, or no reduction possible) so the caller passes it through.
/// Lossy when it fires (drops lines); the caller offloads the original to CCR.
pub fn compress(content: &str) -> Option<Compacted> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= MAX_TOTAL_LINES {
        // Short enough already — the byte budget (if any) will handle it.
        return None;
    }

    // Indices we decide to keep. BTreeSet keeps output in original order.
    let mut keep: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();

    // 1. Summary lines — always keep.
    for (i, line) in lines.iter().enumerate() {
        if is_summary_line(line) {
            keep.insert(i);
        }
    }

    // 2. Errors — first MAX_ERRORS/2 and last MAX_ERRORS/2 by appearance.
    let error_idx: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| severity(l) == Severity::Error)
        .map(|(i, _)| i)
        .collect();
    for &i in select_first_last(&error_idx, MAX_ERRORS).iter() {
        keep.insert(i);
    }

    // 3. Warnings — de-duplicated by normalized text, capped.
    let mut seen_warn: HashSet<String> = HashSet::new();
    let mut warn_kept = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if warn_kept >= MAX_WARNINGS {
            break;
        }
        if severity(line) == Severity::Warning {
            let norm = normalize_for_dedupe(line);
            if seen_warn.insert(norm) {
                keep.insert(i);
                warn_kept += 1;
            }
        }
    }

    // 4. Stack traces — runs of indented / "at "/"#n " frames following an
    //    error, capped in count and per-trace length.
    let mut traces_kept = 0usize;
    let mut i = 0usize;
    while i < lines.len() && traces_kept < MAX_STACK_TRACES {
        if is_stack_frame(lines[i]) {
            let start = i;
            let mut taken = 0usize;
            while i < lines.len() && is_stack_frame(lines[i]) {
                if taken < STACK_TRACE_MAX_LINES {
                    keep.insert(i);
                    taken += 1;
                }
                i += 1;
            }
            if i > start {
                traces_kept += 1;
            }
        } else {
            i += 1;
        }
    }

    if keep.is_empty() {
        // No errors, warnings, stack traces, or summary lines — this almost
        // certainly isn't a log (e.g. generic `shell` output: a file listing,
        // CSV, or a script printing data). Do NOT head/tail-truncate it, which
        // would silently drop the middle of legitimate data. Pass it through to
        // the byte budget instead.
        return None;
    }

    // Enforce the global line cap, keeping the earliest + latest kept lines so
    // the root cause and the final summary both survive.
    let kept_vec: Vec<usize> = keep.iter().copied().collect();
    let kept_vec = if kept_vec.len() > MAX_TOTAL_LINES {
        select_first_last(&kept_vec, MAX_TOTAL_LINES)
    } else {
        kept_vec
    };
    let kept_set: std::collections::BTreeSet<usize> = kept_vec.into_iter().collect();

    // Render with gap markers for runs of dropped lines.
    let mut out = String::with_capacity(content.len() / 2 + 64);
    let mut prev: Option<usize> = None;
    let mut total_dropped = 0usize;
    for &i in &kept_set {
        if let Some(p) = prev {
            let gap = i - p - 1;
            if gap > 0 {
                total_dropped += gap;
                let _ = writeln!(out, "[... {gap} line(s) omitted ...]");
            }
        } else if i > 0 {
            total_dropped += i;
            let _ = writeln!(out, "[... {i} line(s) omitted ...]");
        }
        let _ = writeln!(out, "{}", lines[i]);
        prev = Some(i);
    }
    if let Some(p) = prev {
        let tail = lines.len().saturating_sub(p + 1);
        if tail > 0 {
            total_dropped += tail;
            let _ = writeln!(out, "[... {tail} line(s) omitted ...]");
        }
    }

    if out.len() >= content.len() {
        return None;
    }
    log::debug!(
        "[compaction][logs] kept {} of {} line(s), dropped {}",
        kept_set.len(),
        lines.len(),
        total_dropped,
    );
    Some(Compacted::lossy(out.trim_end().to_string()))
}

/// Choose at most `cap` indices, biased to the first and last by value.
/// `idx` must be ascending. Keeps `ceil(cap/2)` from the front and the rest
/// from the back, preserving order and avoiding duplicates.
fn select_first_last(idx: &[usize], cap: usize) -> Vec<usize> {
    if idx.len() <= cap {
        return idx.to_vec();
    }
    if cap == 0 {
        return Vec::new();
    }
    let head = cap.div_ceil(2);
    let tail = cap - head;
    let mut out: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for &i in idx.iter().take(head) {
        out.insert(i);
    }
    for &i in idx.iter().rev().take(tail) {
        out.insert(i);
    }
    out.into_iter().collect()
}

/// Test/build summary lines worth always keeping.
fn is_summary_line(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    let l = l.trim();
    l.starts_with("test result:")
        || l.starts_with("error: aborting")
        || l.contains(" passed")
        || l.contains(" failed")
        || l.contains("tests passed")
        || l.contains("tests failed")
        || l.contains("failures:")
        || (l.contains("warning") && l.contains("generated"))
        || l.starts_with("error: could not compile")
        || l.starts_with("build failed")
        || l.starts_with("build succeeded")
        || (l.contains("npm") && l.contains("err"))
}

/// A stack-trace frame: leading whitespace + `at `/`#n `/`File "...` etc.
fn is_stack_frame(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    let indented = line.starts_with(' ') || line.starts_with('\t');
    indented
        && (trimmed.starts_with("at ")
            || trimmed.starts_with("File \"")
            || (trimmed.starts_with('#') && trimmed[1..].starts_with(|c: char| c.is_ascii_digit())))
}

/// Normalize a warning line for de-duplication: lowercase and strip digits so
/// "warning at line 12" and "warning at line 88" collapse together.
fn normalize_for_dedupe(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_ascii_digit())
        .collect::<String>()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noisy_log() -> String {
        let mut s = String::new();
        for i in 0..200 {
            let _ = writeln!(s, "   Compiling crate_{i} v0.1.0");
        }
        let _ = writeln!(s, "error[E0382]: borrow of moved value `x`");
        let _ = writeln!(s, "  --> src/main.rs:10:5");
        let _ = writeln!(s, "error: aborting due to previous error");
        let _ = writeln!(s, "test result: FAILED. 3 passed; 1 failed");
        s
    }

    #[test]
    fn keeps_errors_and_summary_drops_noise() {
        let input = noisy_log();
        let out = compress(&input).expect("compresses").text;
        assert!(out.contains("error[E0382]"), "{out}");
        assert!(out.contains("error: aborting"), "{out}");
        assert!(out.contains("test result: FAILED"), "{out}");
        assert!(out.lines().count() <= MAX_TOTAL_LINES + 10);
        assert!(out.len() < input.len());
        assert!(out.contains("omitted"));
    }

    #[test]
    fn dedupes_warnings() {
        let mut s = String::new();
        for i in 0..150 {
            let _ = writeln!(s, "warning: unused variable at line {i}");
        }
        let _ = writeln!(s, "test result: ok. 1 passed; 0 failed");
        let out = compress(&s).expect("compresses").text;
        let warns = out.matches("unused variable").count();
        assert!(warns <= MAX_WARNINGS, "kept {warns} warnings");
    }

    #[test]
    fn non_log_data_passes_through_not_head_tail_truncated() {
        // Generic shell-style output with no errors/warnings/summary: a long
        // listing. Must pass through (None) rather than dropping the middle.
        let mut s = String::new();
        for i in 0..400 {
            let _ = writeln!(s, "/var/data/file_{i:04}.bin\t{i}\trwxr-xr-x");
        }
        assert!(compress(&s).is_none(), "non-log data must not be truncated");
    }

    #[test]
    fn short_log_passes_through() {
        let s = "line1\nline2\nerror: boom\n";
        assert!(compress(s).is_none());
    }

    #[test]
    fn keeps_stack_trace_capped() {
        let mut s = String::new();
        let _ = writeln!(s, "panicked at 'boom'");
        for i in 0..50 {
            let _ = writeln!(s, "    at frame_{i} (src/x.rs:{i})");
        }
        for i in 0..120 {
            let _ = writeln!(s, "info: step {i}");
        }
        let out = compress(&s).expect("compresses").text;
        let frames = out.matches("    at frame_").count();
        assert!(frames <= STACK_TRACE_MAX_LINES, "{frames} frames kept");
    }
}
