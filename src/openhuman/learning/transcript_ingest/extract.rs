//! Heuristic extractor: scans the user/assistant messages of a session
//! transcript and pulls out durable memory candidates plus higher-level
//! reflections.
//!
//! Heuristic-only on purpose — see the module doc for [`super`]. The goal
//! is high-precision extraction of *unmistakable* user statements
//! (preferences, decisions, commitments, unresolved work, explicit
//! self-reflections) so a fresh chat regains continuity without the
//! pipeline ever calling out to a model.
//!
//! ## Filtering rules
//!
//! - User messages only for preferences/commitments — assistant text
//!   *can* echo a preference but is not authoritative.
//! - Decisions and unresolved tasks may come from either side.
//! - Filler messages (under [`MIN_USEFUL_CHARS`] chars after trimming, or
//!   matching [`is_filler`]) are skipped entirely.
//! - Tool messages are never mined — they're high-noise and fully
//!   reconstructable from the transcript itself.

use crate::openhuman::inference::provider::ChatMessage;

use super::types::{CandidateKind, ConversationReflection, Importance, MemoryCandidate};

/// Internal-to-the-module mirror of [`super::types::Provenance`] without
/// `message_indices` — the per-candidate indices are filled in as we
/// match each line.
#[derive(Debug, Clone)]
pub(super) struct Provenance {
    pub thread_id: Option<String>,
    pub transcript_path: String,
    pub transcript_basename: String,
    pub extracted_at: String,
}

/// Below this length a message is treated as filler regardless of its
/// content. Tuned empirically against short acks ("ok", "thanks!", "yes
/// please") that otherwise survive the keyword filters.
pub const MIN_USEFUL_CHARS: usize = 20;

/// Cap individual candidate snippets so a single rambling user turn
/// can't dominate the prompt block on retrieval.
pub const MAX_CANDIDATE_CHARS: usize = 400;

/// User-text patterns that indicate an explicit, durable preference.
/// Case-insensitive substring match; ordering is informational only —
/// the first match wins.
const PREFERENCE_PHRASES: &[&str] = &[
    "i prefer",
    "i'd prefer",
    "i would prefer",
    "i like",
    "i don't like",
    "i hate",
    "i always",
    "i never",
    "please always",
    "please don't",
    "please do not",
    "from now on",
    "going forward",
    "i'd rather",
    "i would rather",
    "i want you to",
];

/// Phrases that indicate a decision (either side may state these).
const DECISION_PHRASES: &[&str] = &[
    "let's go with",
    "let's use",
    "we'll use",
    "we will use",
    "i'll use",
    "i will use",
    "decided to",
    "going with",
    "we're going to use",
    "we picked",
    "we chose",
];

/// Phrases that indicate a commitment by the user (something they
/// promised or planned to do).
const COMMITMENT_PHRASES: &[&str] = &[
    "i'll ",
    "i will ",
    "i'm going to ",
    "i am going to ",
    "i plan to ",
    "i need to ",
];

/// Phrases that indicate an open / unresolved task.
const UNRESOLVED_PHRASES: &[&str] = &[
    "todo",
    "still need to",
    "haven't done",
    "have not done",
    "not done yet",
    "still pending",
    "blocked on",
    "waiting on",
    "follow up on",
    "needs follow-up",
    "next step",
];

/// Phrases that indicate an explicit reflection / improvement signal.
const REFLECTION_PHRASES: &[&str] = &[
    "i realized",
    "i realised",
    "lesson learned",
    "in hindsight",
    "next time",
    "remember that i",
    "remember that we",
    "we keep ",
    "we always end up ",
    "this is the second time",
    "this keeps happening",
];

/// Generic filler patterns that should always be skipped even if a
/// keyword matched — protects against false positives on reactions
/// like "I like that, thanks!".
const FILLER_PATTERNS: &[&str] = &[
    "thanks",
    "thank you",
    "thx",
    "ok cool",
    "sounds good",
    "got it",
];

/// True when `msg` is too short or matches a known filler pattern.
fn is_filler(msg: &str) -> bool {
    let trimmed = msg.trim();
    if trimmed.chars().count() < MIN_USEFUL_CHARS {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    // Pure-filler short messages: the whole message is essentially one
    // of the filler patterns.
    for pat in FILLER_PATTERNS {
        if lower == *pat || lower.trim_end_matches(['.', '!', '?']) == *pat {
            return true;
        }
    }
    false
}

/// Find the first matching phrase from `phrases` in `lower` (already
/// lowercased) and return the substring of `original` starting at that
/// match, truncated to [`MAX_CANDIDATE_CHARS`] and trimmed at the end of
/// the sentence (`.`, `!`, `?`, or newline) where possible.
fn find_phrase_snippet(original: &str, lower: &str, phrases: &[&str]) -> Option<String> {
    let mut best: Option<usize> = None;
    for phrase in phrases {
        if let Some(idx) = lower.find(phrase) {
            best = Some(best.map_or(idx, |b| b.min(idx)));
        }
    }
    let start = best?;
    // Walk back to the start of the containing sentence so the snippet
    // reads naturally (e.g. "I think I prefer X" rather than
    // "I prefer X").
    let prefix = &original[..start];
    let sentence_start = prefix
        .rfind(|c: char| matches!(c, '.' | '!' | '?' | '\n'))
        .map(|i| i + 1)
        .unwrap_or(0);

    let tail = &original[sentence_start..];
    let mut end = tail.len();
    if let Some(rel) = tail.find(|c: char| matches!(c, '\n')) {
        end = end.min(rel);
    }
    if let Some(rel) = tail.find(['.', '!', '?']) {
        // Include the punctuation itself.
        end = end.min(rel + 1);
    }
    let snippet = tail[..end].trim();
    if snippet.is_empty() {
        return None;
    }
    let truncated: String = snippet.chars().take(MAX_CANDIDATE_CHARS).collect();
    Some(truncated)
}

fn make_candidate(
    kind: CandidateKind,
    importance: Importance,
    content: String,
    idx: usize,
    prov: &Provenance,
) -> MemoryCandidate {
    MemoryCandidate {
        kind,
        importance,
        content,
        provenance: super::types::Provenance {
            thread_id: prov.thread_id.clone(),
            transcript_path: prov.transcript_path.clone(),
            transcript_basename: prov.transcript_basename.clone(),
            message_indices: vec![idx],
            extracted_at: prov.extracted_at.clone(),
        },
    }
}

/// Extract durable-fact candidates from a transcript.
pub(super) fn extract_candidates(
    messages: &[ChatMessage],
    prov: &Provenance,
) -> Vec<MemoryCandidate> {
    let mut out: Vec<MemoryCandidate> = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == "tool" || msg.role == "system" {
            continue;
        }
        if is_filler(&msg.content) {
            continue;
        }

        let lower = msg.content.to_ascii_lowercase();
        let is_user = msg.role == "user";

        // Preference / commitment: user-only. High importance — these
        // steer future agent behaviour.
        if is_user {
            if let Some(snippet) = find_phrase_snippet(&msg.content, &lower, PREFERENCE_PHRASES) {
                out.push(make_candidate(
                    CandidateKind::Preference,
                    Importance::High,
                    snippet,
                    idx,
                    prov,
                ));
                continue;
            }
            if let Some(snippet) = find_phrase_snippet(&msg.content, &lower, COMMITMENT_PHRASES) {
                out.push(make_candidate(
                    CandidateKind::Commitment,
                    Importance::Medium,
                    snippet,
                    idx,
                    prov,
                ));
                continue;
            }
        }

        // Decisions and unresolved tasks: either side may state these.
        if let Some(snippet) = find_phrase_snippet(&msg.content, &lower, DECISION_PHRASES) {
            out.push(make_candidate(
                CandidateKind::Decision,
                Importance::High,
                snippet,
                idx,
                prov,
            ));
            continue;
        }
        if let Some(snippet) = find_phrase_snippet(&msg.content, &lower, UNRESOLVED_PHRASES) {
            out.push(make_candidate(
                CandidateKind::UnresolvedTask,
                Importance::Medium,
                snippet,
                idx,
                prov,
            ));
            continue;
        }
    }

    out
}

/// Extract higher-level reflections from a transcript.
///
/// Two sources today:
///
/// 1. **Explicit user reflections** — sentences containing one of the
///    [`REFLECTION_PHRASES`]. Tagged `Importance::High` because the user
///    has signalled they want this remembered.
/// 2. **Repeated-pattern signal** — when the same preference / commitment
///    phrase appears in three or more user messages across the transcript
///    we surface it as a `recurring` reflection so the next session
///    knows this is a stable pattern rather than a one-off remark.
pub(super) fn extract_reflections(
    messages: &[ChatMessage],
    prov: &Provenance,
) -> Vec<ConversationReflection> {
    let mut out: Vec<ConversationReflection> = Vec::new();

    // Explicit reflections from the user.
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role != "user" || is_filler(&msg.content) {
            continue;
        }
        let lower = msg.content.to_ascii_lowercase();
        if let Some(snippet) = find_phrase_snippet(&msg.content, &lower, REFLECTION_PHRASES) {
            out.push(ConversationReflection {
                importance: Importance::High,
                theme: "user_reflection".into(),
                detail: snippet,
                provenance: super::types::Provenance {
                    thread_id: prov.thread_id.clone(),
                    transcript_path: prov.transcript_path.clone(),
                    transcript_basename: prov.transcript_basename.clone(),
                    message_indices: vec![idx],
                    extracted_at: prov.extracted_at.clone(),
                },
            });
        }
    }

    // Recurring-preference detection: count how many user turns mention
    // any preference phrase. If ≥3, emit one recurring reflection
    // citing all matching message indices.
    let mut recurring_indices: Vec<usize> = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role != "user" || is_filler(&msg.content) {
            continue;
        }
        let lower = msg.content.to_ascii_lowercase();
        if PREFERENCE_PHRASES.iter().any(|p| lower.contains(p)) {
            recurring_indices.push(idx);
        }
    }
    if recurring_indices.len() >= 3 {
        out.push(ConversationReflection {
            importance: Importance::Medium,
            theme: "recurring_preferences".into(),
            detail: format!(
                "User stated personal preferences in {} messages this session — treat as a stable pattern, not a one-off.",
                recurring_indices.len()
            ),
            provenance: super::types::Provenance {
                thread_id: prov.thread_id.clone(),
                transcript_path: prov.transcript_path.clone(),
                transcript_basename: prov.transcript_basename.clone(),
                message_indices: recurring_indices,
                extracted_at: prov.extracted_at.clone(),
            },
        });
    }

    out
}

#[cfg(test)]
mod inline_tests {
    use super::*;

    fn prov() -> Provenance {
        Provenance {
            thread_id: Some("thr_abc".into()),
            transcript_path: "/tmp/session_raw/123_main.jsonl".into(),
            transcript_basename: "123_main.jsonl".into(),
            extracted_at: "2026-05-09T12:00:00Z".into(),
        }
    }

    #[test]
    fn skips_short_filler() {
        assert!(is_filler("ok"));
        assert!(is_filler("thanks!"));
        assert!(is_filler("hi"));
        assert!(!is_filler("I prefer Postgres for this kind of thing."));
    }

    #[test]
    fn extracts_user_preference_as_high() {
        let msgs = vec![ChatMessage::user(
            "I prefer Postgres over MySQL for new services.",
        )];
        let cands = extract_candidates(&msgs, &prov());
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].kind, CandidateKind::Preference);
        assert_eq!(cands[0].importance, Importance::High);
        assert!(cands[0].content.contains("Postgres"));
    }

    #[test]
    fn does_not_extract_preference_from_assistant() {
        let msgs = vec![ChatMessage::assistant(
            "You said earlier that I prefer Postgres for these.",
        )];
        let cands = extract_candidates(&msgs, &prov());
        assert!(
            cands.iter().all(|c| c.kind != CandidateKind::Preference),
            "assistant text must not produce Preference: {:?}",
            cands,
        );
    }

    #[test]
    fn extracts_decision_from_either_side() {
        let msgs = vec![
            ChatMessage::user("Let's go with Postgres for the metadata store."),
            ChatMessage::assistant("Sure, going with Postgres."),
        ];
        let cands = extract_candidates(&msgs, &prov());
        let decisions: Vec<_> = cands
            .iter()
            .filter(|c| c.kind == CandidateKind::Decision)
            .collect();
        assert!(
            !decisions.is_empty(),
            "should extract at least one decision"
        );
    }

    #[test]
    fn extracts_unresolved_task() {
        let msgs = vec![ChatMessage::user(
            "Still need to migrate the old auth service before Friday.",
        )];
        let cands = extract_candidates(&msgs, &prov());
        assert!(cands
            .iter()
            .any(|c| c.kind == CandidateKind::UnresolvedTask));
    }

    #[test]
    fn captures_reflection_with_provenance_indices() {
        let msgs = vec![
            ChatMessage::user("Hello, can you help with the deploy?"),
            ChatMessage::assistant("Sure, what's broken?"),
            ChatMessage::user(
                "I realized our staging cluster is the bottleneck — \
                 next time let's pre-warm it.",
            ),
        ];
        let refls = extract_reflections(&msgs, &prov());
        assert_eq!(refls.len(), 1);
        assert_eq!(refls[0].theme, "user_reflection");
        assert_eq!(refls[0].provenance.message_indices, vec![2]);
    }
}
