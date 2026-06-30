//! Build/test/lint log compressor.
//!
//! Two paths, chosen by whether the content is *command* output:
//!
//! - **Command output** (the [`CompressInput`] carries a derived command/argv):
//!   run the 100-rule reduction engine ([`reduce_execution_with_rules`]). This
//!   is TokenJuice's original behaviour — git/cargo/npm/docker-aware rules with
//!   failure preservation.
//! - **Non-command logs** (a blob detected as a log with no command context):
//!   the signal-based keep-failures/drop-noise compressor, a clean-room port of
//!   Headroom's `LogCompressor` (Apache-2.0).
//!
//! The signal path declines (returns `None`) when a blob has no error/warning/
//! summary/stack signal at all — that almost certainly isn't a log (a file
//! listing, CSV, generated data) and must not be head/tail truncated.

use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::fmt::Write as _;

use super::signals::{severity, Severity};
use super::Compressor;
use crate::openhuman::tokenjuice::reduce::reduce_execution_with_rules;
use crate::openhuman::tokenjuice::rules::load_builtin_rules;
use crate::openhuman::tokenjuice::types::{
    CompiledRule, CompressInput, CompressOptions, CompressOutput, CompressorKind, ReduceOptions,
    ToolExecutionInput,
};

pub const MAX_ERRORS: usize = 10;
pub const MAX_WARNINGS: usize = 5;
pub const MAX_STACK_TRACES: usize = 3;
pub const STACK_TRACE_MAX_LINES: usize = 20;
pub const MAX_TOTAL_LINES: usize = 100;

static BUILTIN_RULES: Lazy<Vec<CompiledRule>> = Lazy::new(load_builtin_rules);

pub struct LogCompressor;

#[async_trait]
impl Compressor for LogCompressor {
    fn kind(&self) -> CompressorKind {
        CompressorKind::Log
    }

    async fn compress(
        &self,
        input: &CompressInput<'_>,
        opts: &CompressOptions,
    ) -> Option<CompressOutput> {
        let has_command =
            input.command.is_some() || input.argv.as_ref().is_some_and(|a| !a.is_empty());
        if has_command {
            compress_command(input, opts)
        } else {
            compress_signal(input.content)
        }
    }
}

/// Command output → run the rule engine, tagged as [`CompressorKind::Log`].
fn compress_command(input: &CompressInput<'_>, opts: &CompressOptions) -> Option<CompressOutput> {
    run_rule_engine(input, opts, CompressorKind::Log)
}

/// The generic fallback path: same rule engine, tagged [`CompressorKind::Generic`].
/// Exposed for [`super::generic::GenericCompressor`] so the router can fall back
/// to head/tail summarisation of command output without re-implementing it.
pub fn compress_command_fallback(
    input: &CompressInput<'_>,
    opts: &CompressOptions,
) -> Option<CompressOutput> {
    run_rule_engine(input, opts, CompressorKind::Generic)
}

/// Run the 100-rule reduction engine over command output, reporting `kind`.
/// Returns `None` when reduction wouldn't shrink the payload.
fn run_rule_engine(
    input: &CompressInput<'_>,
    opts: &CompressOptions,
    kind: CompressorKind,
) -> Option<CompressOutput> {
    let exec = ToolExecutionInput {
        tool_name: input
            .hint
            .source_tool
            .clone()
            .unwrap_or_else(|| "shell".to_string()),
        command: input.command.clone(),
        argv: input.argv.clone(),
        stdout: Some(input.content.to_string()),
        exit_code: input.exit_code,
        ..Default::default()
    };
    let reduce_opts = ReduceOptions {
        max_inline_chars: opts.max_inline_chars,
        ..Default::default()
    };
    let result = reduce_execution_with_rules(exec, &BUILTIN_RULES, &reduce_opts);
    if result.inline_text.len() >= input.content.len() {
        return None;
    }
    let rule_label = result
        .classification
        .matched_reducer
        .unwrap_or(result.classification.family);
    log::debug!(
        "[tokenjuice][log] command rule={} kind={} {} -> {} bytes",
        rule_label,
        kind.as_str(),
        input.content.len(),
        result.inline_text.len()
    );
    Some(CompressOutput::lossy(result.inline_text, kind))
}

/// Signal-based log compression for non-command blobs detected as logs.
pub fn compress_signal(content: &str) -> Option<CompressOutput> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= MAX_TOTAL_LINES {
        return None;
    }

    let mut keep: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();

    for (i, line) in lines.iter().enumerate() {
        if is_summary_line(line) {
            keep.insert(i);
        }
    }

    let error_idx: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| severity(l) == Severity::Error)
        .map(|(i, _)| i)
        .collect();
    for &i in select_first_last(&error_idx, MAX_ERRORS).iter() {
        keep.insert(i);
    }

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
        // Not a log (no signal) — never head/tail truncate legitimate data.
        return None;
    }

    let kept_vec: Vec<usize> = keep.iter().copied().collect();
    let kept_vec = if kept_vec.len() > MAX_TOTAL_LINES {
        select_first_last(&kept_vec, MAX_TOTAL_LINES)
    } else {
        kept_vec
    };
    let kept_set: std::collections::BTreeSet<usize> = kept_vec.into_iter().collect();

    let mut out = String::with_capacity(content.len() / 2 + 64);
    let mut prev: Option<usize> = None;
    for &i in &kept_set {
        if let Some(p) = prev {
            let gap = i - p - 1;
            if gap > 0 {
                let _ = writeln!(out, "[... {gap} line(s) omitted ...]");
            }
        } else if i > 0 {
            let _ = writeln!(out, "[... {i} line(s) omitted ...]");
        }
        let _ = writeln!(out, "{}", lines[i]);
        prev = Some(i);
    }
    if let Some(p) = prev {
        let tail = lines.len().saturating_sub(p + 1);
        if tail > 0 {
            let _ = writeln!(out, "[... {tail} line(s) omitted ...]");
        }
    }

    if out.len() >= content.len() {
        return None;
    }
    log::debug!(
        "[tokenjuice][log] signal kept {} of {} line(s)",
        kept_set.len(),
        lines.len(),
    );
    Some(CompressOutput::lossy(
        out.trim_end().to_string(),
        CompressorKind::Log,
    ))
}

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
    fn signal_keeps_errors_and_summary_drops_noise() {
        let input = noisy_log();
        let out = compress_signal(&input).expect("compresses").text;
        assert!(out.contains("error[E0382]"), "{out}");
        assert!(out.contains("error: aborting"), "{out}");
        assert!(out.contains("test result: FAILED"), "{out}");
        assert!(out.len() < input.len());
        assert!(out.contains("omitted"));
    }

    #[test]
    fn non_log_data_passes_through() {
        let mut s = String::new();
        for i in 0..400 {
            let _ = writeln!(s, "/var/data/file_{i:04}.bin\t{i}\trwxr-xr-x");
        }
        assert!(compress_signal(&s).is_none());
    }
}
