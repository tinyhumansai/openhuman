//! Explainable, deterministic, no-egress code location over authorized bytes.
//!
//! Inspired by jevex/blink search packets without introducing another index,
//! credential, model call, or store. It only ranks observed (hash-anchored)
//! source files; it NEVER asserts that an unobserved file is irrelevant.
use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::source_graph::SourceDocument;

const MAX_RESULTS: usize = 5;
const MAX_EVIDENCE_LINES: usize = 3;
const MAX_LINE_CHARS: usize = 160;
const MAX_JSON_BYTES: usize = 26_000;

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "code", "do", "does", "file", "files",
    "find", "for", "from", "how", "i", "in", "is", "it", "me", "of", "on", "or", "please", "show",
    "the", "this", "to", "was", "were", "what", "when", "where", "which", "who", "why", "with",
    "would", "you",
];

/// Split literal terms at word and camel-case boundaries while preserving
/// first-seen order. A sorted set is only appropriate for later intersections,
/// never for deciding which query requirements survive a strict term budget.
fn tokenize_ordered(input: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut last_lower = false;
    for ch in input.chars() {
        if !ch.is_alphanumeric() {
            if (2..=64).contains(&current.chars().count()) {
                pieces.push(current.to_lowercase());
            }
            current.clear();
            last_lower = false;
            continue;
        }
        if last_lower && ch.is_uppercase() && !current.is_empty() {
            if (2..=64).contains(&current.chars().count()) {
                pieces.push(current.to_lowercase());
            }
            current.clear();
        }
        if current.chars().count() < 64 {
            current.push(ch);
        }
        last_lower = ch.is_lowercase();
    }
    if (2..=64).contains(&current.chars().count()) {
        pieces.push(current.to_lowercase());
    }
    pieces
}

fn tokenize(input: &str) -> BTreeSet<String> {
    tokenize_ordered(input).into_iter().collect()
}

fn query_terms(input: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let ordered: Vec<String> = tokenize_ordered(input)
        .into_iter()
        .filter(|token| !STOPWORDS.contains(&token.as_str()))
        .filter(|token| seen.insert(token.clone()))
        .collect();
    const MAX_TERMS: usize = 16;
    if ordered.len() <= MAX_TERMS {
        return ordered.into_iter().collect();
    }
    // Retain the original high-level intent AND late specific constraints.
    // Alphabetically taking the first 16 drops key requirements like a final
    // class/method name simply because they sort after generic words.
    ordered
        .iter()
        .take(MAX_TERMS / 2)
        .chain(ordered.iter().skip(ordered.len() - MAX_TERMS / 2))
        .cloned()
        .collect()
}

/// Locate a whole lexical query token, including camel-case parts, using
/// byte offsets at UTF-8 character boundaries. A prefix inside "superuser"
/// must not count as a match for "user".
fn first_matched_token_byte(line: &str, query: &BTreeSet<String>) -> Option<usize> {
    let mut begin: Option<usize> = None;
    let mut prior_lower = false;
    let matching = |start: usize, end: usize| {
        let value = &line[start..end];
        (2..=64).contains(&value.chars().count()) && query.contains(&value.to_lowercase())
    };
    for (byte, ch) in line.char_indices() {
        if !ch.is_alphanumeric() {
            if let Some(start) = begin {
                if matching(start, byte) {
                    return Some(start);
                }
            }
            begin = None;
            prior_lower = false;
            continue;
        }
        if prior_lower && ch.is_uppercase() {
            if let Some(start) = begin {
                if matching(start, byte) {
                    return Some(start);
                }
            }
            begin = Some(byte);
        } else if begin.is_none() {
            begin = Some(byte);
        }
        prior_lower = ch.is_lowercase();
    }
    if let Some(start) = begin {
        if matching(start, line.len()) {
            return Some(start);
        }
    }
    None
}

fn evidence(doc: &SourceDocument, query: &BTreeSet<String>) -> (BTreeSet<String>, Vec<Value>) {
    let mut all_found = BTreeSet::new();
    let mut spans = Vec::new();
    for (line_number, line) in doc.content.lines().enumerate() {
        let tokens = tokenize(line);
        let hits: Vec<String> = query.intersection(&tokens).cloned().collect();
        if hits.is_empty() {
            continue;
        }
        all_found.extend(hits.iter().cloned());
        if spans.len() < MAX_EVIDENCE_LINES {
            let line_chars = line.chars().count();
            let hit_byte = first_matched_token_byte(line, query).unwrap_or(0);
            let hit_column = line[..hit_byte].chars().count();
            let start = if line_chars > MAX_LINE_CHARS {
                hit_column
                    .saturating_sub(MAX_LINE_CHARS / 3)
                    .min(line_chars - MAX_LINE_CHARS)
            } else {
                0
            };
            spans.push(json!({
                "line": line_number + 1,
                "column_start": start + 1,
                "line_char_count": line_chars,
                "excerpt_truncated": start > 0 || line_chars > MAX_LINE_CHARS,
                "text": line.chars().skip(start).take(MAX_LINE_CHARS).collect::<String>(),
                "matched_terms": hits
            }));
        }
    }
    (all_found, spans)
}

/// Produces bounded, source-hash-anchored *candidate* ranges, not verified
/// source-of-truth. Never performs I/O. Inputs must already have passed the
/// existing SourceEvidenceTool workspace/security/file-size policy.
pub(super) fn rank_sources(docs: &[SourceDocument], query: &str, limit: usize) -> Value {
    let terms = query_terms(query);
    let mut scores: Vec<(usize, String, Value)> = Vec::new();

    if !terms.is_empty() {
        for doc in docs {
            let path_tokens = tokenize(&doc.path);
            let path_hits: BTreeSet<String> = terms.intersection(&path_tokens).cloned().collect();
            let (body_hits, spans) = evidence(doc, &terms);
            let matched: BTreeSet<String> = path_hits.union(&body_hits).cloned().collect();
            if matched.is_empty() {
                continue;
            }
            // Explainable integer heuristic, not calibrated probability.
            // Path locality and distinct term coverage outrank repeated
            // occurrences in large generic documentation files.
            let score = path_hits.len() * 12 + body_hits.len() * 5 + matched.len() * 3;
            scores.push((
                score,
                doc.path.clone(),
                json!({
                    "path": doc.path,
                    "source_hash": doc.source_hash,
                    "score": score,
                    "matched_terms": matched,
                    "evidence": spans,
                    "evidence_verified": false
                }),
            ));
        }
    }
    scores.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let total_candidates = scores.len();
    let candidates: Vec<Value> = scores
        .into_iter()
        .take(limit.clamp(1, MAX_RESULTS))
        .map(|(_, _, item)| item)
        .collect();

    // Host tool outputs are plain text and may otherwise be clipped into
    // syntactically-invalid JSON. Keep a coherent, explicitly partial result.
    let mut response = json!({
        "operation": "locate",
        "ranking_kind": "local_lexical_heuristic",
        "results": candidates,
        "ranked_candidates": total_candidates,
        "effective_query_terms": terms.len(),
        "source_of_truth_verified": false,
        "no_match_is_exhaustive": false,
        "snapshot_atomic": false
    });
    while response.to_string().len() > MAX_JSON_BYTES {
        let Some(items) = response["results"].as_array_mut() else {
            break;
        };
        if items.pop().is_none() {
            break;
        }
        response["ranking_truncated"] = json!(true);
    }
    response
}

#[cfg(test)]
#[path = "source_locate_tests.rs"]
mod tests;
