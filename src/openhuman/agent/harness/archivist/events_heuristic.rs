//! Heuristic event extraction from segment prose.
//!
//! # Why this is the host's and not the engine's
//!
//! Which sentence patterns count as a "decision" or a "commitment" is policy
//! about what the product surfaces, exactly like the preference lanes and the
//! redaction hash before it. The engine used to own an identical helper and
//! the archivist called it; that was an engine dependency taken on for a page
//! of string matching. The engine keeps its copy for its own pipelines — the
//! two are independent by design, and nothing compares their outputs.
//!
//! Ported **verbatim** from the engine's classifier — same pattern lists, same
//! ≥5-char sentence floor, same one-match-per-category-with-dedupe rule — so
//! the migrated caller stores exactly the events it always stored.

use crate::openhuman::memory::api::provider::EventKind;

/// What a matched sentence records. Mirrors the contract's [`EventKind`],
/// with the mapping kept explicit so a wire rename cannot silently re-label
/// stored events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExtractedEventKind {
    Fact,
    Decision,
    Commitment,
    Preference,
}

impl ExtractedEventKind {
    /// The contract kind this extraction records as.
    pub(super) fn contract(self) -> EventKind {
        match self {
            Self::Fact => EventKind::Fact,
            Self::Decision => EventKind::Decision,
            Self::Commitment => EventKind::Commitment,
            Self::Preference => EventKind::Preference,
        }
    }
}

/// Patterns that indicate a decision was made.
const DECISION_PATTERNS: &[&str] = &[
    "let's go with",
    "lets go with",
    "i've decided",
    "ive decided",
    "i decided",
    "we decided",
    "we agreed",
    "the decision is",
    "going with",
    "we'll use",
    "well use",
    "i'll use",
    "chosen to",
    "i choose",
    "we choose",
];

/// Patterns that indicate a commitment or deadline.
const COMMITMENT_PATTERNS: &[&str] = &[
    "by friday",
    "by monday",
    "by tuesday",
    "by wednesday",
    "by thursday",
    "by saturday",
    "by sunday",
    "by tomorrow",
    "by next week",
    "by end of",
    "deadline is",
    "due date",
    "i will",
    "i'll do",
    "ill do",
    "i promise",
    "i commit",
    "we need to finish",
    "scheduled for",
    "plan to",
    "planning to",
];

/// Patterns that indicate a preference.
const PREFERENCE_PATTERNS: &[&str] = &[
    "i prefer",
    "i like",
    "i love",
    "i hate",
    "i dislike",
    "i always",
    "i never",
    "my favorite",
    "my favourite",
    "i usually",
    "i tend to",
    "i'm used to",
    "im used to",
];

/// Patterns that indicate a personal fact.
const FACT_PATTERNS: &[&str] = &[
    "i'm based in",
    "im based in",
    "i live in",
    "i work at",
    "i work for",
    "my name is",
    "i'm a",
    "im a",
    "i am a",
    "my role is",
    "i've been",
    "ive been",
    "i have been",
    "i'm from",
    "im from",
    "my timezone",
    "my time zone",
];

/// Extract events from text using heuristic pattern matching.
/// Returns a list of (event_kind, matched_sentence) pairs.
pub(super) fn extract_events_heuristic(text: &str) -> Vec<(ExtractedEventKind, String)> {
    let mut events = Vec::new();

    // Split into sentences (rough heuristic).
    let sentences: Vec<&str> = text
        .split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| s.len() > 5)
        .collect();

    for sentence in sentences {
        let lower = sentence.to_lowercase();

        // Check each pattern category.
        for pattern in DECISION_PATTERNS {
            if lower.contains(pattern) {
                events.push((ExtractedEventKind::Decision, sentence.to_string()));
                break;
            }
        }
        for pattern in COMMITMENT_PATTERNS {
            if lower.contains(pattern) {
                // Avoid duplicate if already matched as decision.
                if !events.iter().any(|(_, s)| s == sentence) {
                    events.push((ExtractedEventKind::Commitment, sentence.to_string()));
                }
                break;
            }
        }
        for pattern in PREFERENCE_PATTERNS {
            if lower.contains(pattern) {
                if !events.iter().any(|(_, s)| s == sentence) {
                    events.push((ExtractedEventKind::Preference, sentence.to_string()));
                }
                break;
            }
        }
        for pattern in FACT_PATTERNS {
            if lower.contains(pattern) {
                if !events.iter().any(|(_, s)| s == sentence) {
                    events.push((ExtractedEventKind::Fact, sentence.to_string()));
                }
                break;
            }
        }
    }

    events
}
