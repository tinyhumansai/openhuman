//! Search-results compressor (relevance ranking).
//!
//! grep / ripgrep output is `path:line:body` matches. For very large result
//! sets the compressor groups matches by file, ranks them, keeps the top-K per
//! file plus a `[+N more in <file>]` tally, and (via the router) offloads the
//! full result set to CCR so the complete list is one `retrieve` away.
//!
//! Ranking: when the caller supplies a `query` in the [`ContentHint`], matches
//! are scored by query-term density in the body; otherwise by body length /
//! uniqueness (longer, more-distinctive lines first), with importance signals
//! (error/TODO) always boosted. Group/file order is preserved (first-seen).
//!
//! NOTE: this compressor is gated by `opts.search_enabled` in the router. It is
//! a behaviour change from the historical "never compact grep" stance — kept
//! lossless by the CCR offload — and is enabled by default per project decision.

use async_trait::async_trait;
use std::fmt::Write as _;

use super::signals::line_score;
use super::Compressor;
use crate::openhuman::tokenjuice::detect::parse_search_line;
use crate::openhuman::tokenjuice::types::{
    CompressInput, CompressOptions, CompressOutput, CompressorKind,
};

/// Only compress result sets with more than this many matching lines.
pub const MIN_MATCHES: usize = 40;
/// Matches kept per file before the "+N more" tally.
pub const TOP_K_PER_FILE: usize = 5;

pub struct SearchCompressor;

#[async_trait]
impl Compressor for SearchCompressor {
    fn kind(&self) -> CompressorKind {
        CompressorKind::Search
    }

    async fn compress(
        &self,
        input: &CompressInput<'_>,
        _opts: &CompressOptions,
    ) -> Option<CompressOutput> {
        compress(input.content, input.hint.query.as_deref())
    }
}

struct Match<'a> {
    line_no: u64,
    body: &'a str,
    score: f32,
    raw: &'a str,
}

/// Compress search output. `query` (when known) ranks matches by term density.
pub fn compress(content: &str, query: Option<&str>) -> Option<CompressOutput> {
    // Preserve any non-match preamble/summary lines (e.g. "80 match(es)") and
    // group match lines by file in first-seen order.
    let mut preamble: Vec<&str> = Vec::new();
    let mut files: Vec<(&str, Vec<Match<'_>>)> = Vec::new();
    let mut match_count = 0usize;

    let query_terms: Vec<String> = query
        .map(|q| {
            q.split_whitespace()
                .map(|t| t.to_ascii_lowercase())
                .filter(|t| t.len() >= 2)
                .collect()
        })
        .unwrap_or_default();

    for line in content.lines() {
        match parse_search_line(line) {
            Some((path, line_no, body)) => {
                match_count += 1;
                let score = score_match(body, &query_terms);
                let m = Match {
                    line_no,
                    body,
                    score,
                    raw: line,
                };
                if let Some((_, v)) = files.iter_mut().find(|(p, _)| *p == path) {
                    v.push(m);
                } else {
                    files.push((path, vec![m]));
                }
            }
            None => {
                if !line.trim().is_empty() && files.is_empty() {
                    // Only keep preamble that appears before any match.
                    preamble.push(line);
                }
            }
        }
    }

    if match_count < MIN_MATCHES || files.is_empty() {
        return None;
    }

    let mut out = String::with_capacity(content.len() / 2 + 64);
    for line in &preamble {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(
        out,
        "[search: {} match(es) across {} file(s) · top {} per file · full set via retrieve footer]",
        match_count,
        files.len(),
        TOP_K_PER_FILE
    );

    for (path, mut matches) in files {
        let total = matches.len();
        if total <= TOP_K_PER_FILE {
            // Keep all in original (line-number) order.
            matches.sort_by_key(|m| m.line_no);
            for m in &matches {
                let _ = writeln!(out, "{}", m.raw);
            }
            continue;
        }
        // Rank by score (desc), keep top-K, then re-sort kept by line number so
        // the output reads top-to-bottom within the file.
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut kept: Vec<&Match<'_>> = matches.iter().take(TOP_K_PER_FILE).collect();
        kept.sort_by_key(|m| m.line_no);
        for m in &kept {
            let _ = writeln!(out, "{}:{}:{}", path, m.line_no, m.body);
        }
        let _ = writeln!(
            out,
            "[+{} more match(es) in {path}]",
            total - TOP_K_PER_FILE
        );
    }

    let out = out.trim_end().to_string();
    if out.len() >= content.len() {
        return None;
    }
    log::debug!(
        "[tokenjuice][search] {} matches -> {} bytes (from {} bytes)",
        match_count,
        out.len(),
        content.len()
    );
    Some(CompressOutput::lossy(out, CompressorKind::Search))
}

/// Score a match body. With query terms, density of those terms dominates;
/// otherwise distinctiveness (length) with an importance bump for error/TODO.
fn score_match(body: &str, query_terms: &[String]) -> f32 {
    let importance = line_score(body);
    if query_terms.is_empty() {
        // No query: favour longer, more-distinctive lines, plus importance.
        let len_score = (body.trim().len() as f32 / 80.0).min(1.0);
        return importance.max(0.2 + 0.8 * len_score);
    }
    let lower = body.to_ascii_lowercase();
    let hits = query_terms.iter().filter(|t| lower.contains(*t)).count();
    let density = hits as f32 / query_terms.len() as f32;
    importance.max(density)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big_results() -> String {
        let mut s = String::from("120 match(es); scanned 3 file(s)\n");
        for i in 0..60 {
            let _ = writeln!(s, "src/a.rs:{i}:let value_{i} = compute_long_name_{i}();");
        }
        for i in 0..60 {
            let _ = writeln!(s, "src/b.rs:{i}:fn helper_function_number_{i}() {{}}");
        }
        s
    }

    #[test]
    fn keeps_top_k_per_file_and_tally() {
        let input = big_results();
        let out = compress(&input, None).expect("compresses").text;
        assert!(out.contains("more match(es) in src/a.rs"), "{out}");
        assert!(out.contains("more match(es) in src/b.rs"));
        // Preamble survives.
        assert!(out.contains("120 match(es)"));
        assert!(out.len() < input.len());
    }

    #[test]
    fn query_ranks_relevant_matches() {
        let mut s = String::new();
        for i in 0..50 {
            let body = if i == 7 {
                "the special needle token appears here".to_string()
            } else {
                format!("ordinary line content number {i}")
            };
            let _ = writeln!(s, "src/x.rs:{i}:{body}");
        }
        let out = compress(&s, Some("needle token")).expect("compresses").text;
        assert!(
            out.contains("special needle token"),
            "ranked-in match missing:\n{out}"
        );
    }

    #[test]
    fn small_result_set_passes_through() {
        let s = "a.rs:1:hit\nb.rs:2:hit\n";
        assert!(compress(s, None).is_none());
    }
}
