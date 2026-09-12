//! When one conversation segment ends and the next begins — and what to call it
//! when no model is available to say.
//!
//! # Why this is host-side
//!
//! These functions came from the engine's `segments` module, and unlike their
//! neighbours there they never touched the database: every one of them is a
//! pure function over values the caller already holds. That is what makes the
//! split obvious. Persisting a segment is storage; deciding *that a segment
//! should end* is a product judgement about what a conversation is — how long a
//! pause has to be before the subject has moved on, how many turns is too many,
//! which phrases signal a change of topic. A second engine has no business
//! holding an opinion on any of it, and the host that renders these segments to
//! the user is the only thing that can tune them against what users actually
//! see.
//!
//! The rest of the archivist's engine calls became the `Episodic` contract
//! family, which persists what this module decides.
//!
//! # The thresholds are unchanged, deliberately
//!
//! Every value here — the ten-minute gap, the 0.4 similarity floor, the
//! twenty-turn cap, the marker list, the 200-character bookends — is carried
//! over verbatim from the engine. This is a move, not a retune: changing
//! behaviour in the same step that changes where the behaviour lives would make
//! any resulting regression impossible to attribute. Tune them afterwards,
//! against real segments.

use serde::{Deserialize, Serialize};

/// Thresholds governing when a new turn starts a new segment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundaryConfig {
    /// Maximum gap (seconds) between turns before forcing a new segment.
    pub max_time_gap_secs: f64,
    /// Minimum cosine similarity between the turn's embedding and the
    /// segment's centroid. Below this, the subject is taken to have drifted.
    pub min_cosine_similarity: f32,
    /// Maximum turns in one segment before a boundary is forced.
    pub max_turns_per_segment: i32,
}

impl Default for BoundaryConfig {
    fn default() -> Self {
        Self {
            max_time_gap_secs: 600.0, // 10 minutes
            min_cosine_similarity: 0.4,
            max_turns_per_segment: 20,
        }
    }
}

/// Whether a new turn continues the current segment or opens a new one.
#[derive(Clone, Debug, PartialEq)]
pub enum BoundaryDecision {
    /// Keep accumulating into the current segment.
    Continue,
    /// Close the current segment and start a new one.
    Boundary(BoundaryReason),
}

/// Why a boundary was declared. Carried so the decision can be logged and
/// explained rather than just obeyed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryReason {
    /// Too long a pause since the previous turn.
    TimeGap,
    /// The turn's embedding drifted away from the segment centroid.
    EmbeddingDrift,
    /// The turn opened with a phrase that announces a change of subject.
    ExplicitMarker,
    /// The segment is already at its turn cap.
    TurnCountExceeded,
}

impl std::fmt::Display for BoundaryReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimeGap => write!(f, "time_gap"),
            Self::EmbeddingDrift => write!(f, "embedding_drift"),
            Self::ExplicitMarker => write!(f, "explicit_marker"),
            Self::TurnCountExceeded => write!(f, "turn_count_exceeded"),
        }
    }
}

/// Phrases that announce a change of subject.
///
/// Deliberately literal and English-only, matched case-insensitively as
/// substrings. It is a cheap first-pass signal that runs before the embedding
/// comparison, not a claim to detect topic change in general — the drift check
/// below is what catches the cases this list cannot.
const TOPIC_CHANGE_MARKERS: &[&str] = &[
    "now let's",
    "now lets",
    "switching to",
    "different topic",
    "moving on to",
    "let's move on",
    "lets move on",
    "can you help me with",
    "new question",
    "unrelated but",
    "changing subject",
    "on another note",
    "anyway,",
    "by the way,",
    "btw,",
];

/// Decide whether `new_turn` belongs to `current_segment`.
///
/// The four checks run cheapest-first, and each returns immediately: turn count
/// and time gap are arithmetic on values already in hand, the marker scan is a
/// handful of substring searches, and only the embedding comparison touches
/// vectors. A turn that trips an earlier check never pays for a later one.
#[must_use]
pub fn detect_boundary(
    config: &BoundaryConfig,
    current_segment: &SegmentBoundaryState,
    new_turn_timestamp: f64,
    new_turn_content: &str,
    new_turn_embedding: Option<&[f32]>,
) -> BoundaryDecision {
    // 1. Turn count exceeded.
    if current_segment.turn_count >= config.max_turns_per_segment {
        tracing::debug!(
            "[segments] boundary: turn count {} >= {}",
            current_segment.turn_count,
            config.max_turns_per_segment
        );
        return BoundaryDecision::Boundary(BoundaryReason::TurnCountExceeded);
    }

    // 2. Time gap. Falls back to the segment's start when no turn has been
    // appended yet, so a one-turn segment is measured from its own beginning.
    let last_timestamp = current_segment
        .end_timestamp
        .unwrap_or(current_segment.start_timestamp);
    let gap = new_turn_timestamp - last_timestamp;
    if gap > config.max_time_gap_secs {
        tracing::debug!(
            "[segments] boundary: time gap {gap:.0}s > {}s",
            config.max_time_gap_secs
        );
        return BoundaryDecision::Boundary(BoundaryReason::TimeGap);
    }

    // 3. Explicit topic-change markers.
    let content_lower = new_turn_content.to_lowercase();
    for marker in TOPIC_CHANGE_MARKERS {
        if content_lower.contains(marker) {
            tracing::debug!("[segments] boundary: explicit marker '{marker}'");
            return BoundaryDecision::Boundary(BoundaryReason::ExplicitMarker);
        }
    }

    // 4. Embedding drift. Skipped unless both vectors exist and agree on
    // length: comparing across embedding spaces would produce a meaningless
    // similarity, and treating that as drift would split segments at random.
    if let (Some(segment_emb), Some(turn_emb)) =
        (current_segment.embedding.as_deref(), new_turn_embedding)
    {
        if !segment_emb.is_empty() && segment_emb.len() == turn_emb.len() {
            let similarity = cosine_similarity(segment_emb, turn_emb);
            if similarity < config.min_cosine_similarity {
                tracing::debug!(
                    "[segments] boundary: embedding drift (sim={similarity:.3} < {})",
                    config.min_cosine_similarity
                );
                return BoundaryDecision::Boundary(BoundaryReason::EmbeddingDrift);
            }
        }
    }

    BoundaryDecision::Continue
}

/// The part of a segment boundary detection reads.
///
/// A narrow view rather than the whole
/// [`ConversationSegment`](tinymemory_api::provider::episodic::ConversationSegment)
/// because the decision depends on four fields, and naming them makes it
/// checkable that nothing else influences it.
#[derive(Clone, Debug, Default)]
pub struct SegmentBoundaryState {
    /// Turns accumulated so far.
    pub turn_count: i32,
    /// When the segment began.
    pub start_timestamp: f64,
    /// When its most recent turn arrived, if any.
    pub end_timestamp: Option<f64>,
    /// The segment's running centroid, if it has one.
    pub embedding: Option<Vec<f32>>,
}

/// Fold a new vector into a running centroid, returning the incremental mean.
///
/// Returns `new_embedding` unchanged when there is no usable centroid yet or
/// the dimensions disagree — the same guard as the drift check, for the same
/// reason.
#[must_use]
pub fn incremental_mean_embedding(
    current_centroid: &[f32],
    new_embedding: &[f32],
    count: usize,
) -> Vec<f32> {
    if current_centroid.is_empty() || current_centroid.len() != new_embedding.len() {
        return new_embedding.to_vec();
    }
    current_centroid
        .iter()
        .zip(new_embedding.iter())
        .map(|(c, n)| c + (n - c) / (count as f32 + 1.0))
        .collect()
}

/// A summary composed from the segment's first and last turns.
///
/// Used when no model is available or the recap call failed. It is a bookend,
/// not a summary, and reads like one on purpose: a caller comparing this
/// against a real recap should be able to tell them apart.
#[must_use]
pub fn fallback_summary(first_content: &str, last_content: &str, turn_count: i32) -> String {
    let first_truncated = truncate_utf8_safe(first_content, 200);
    let last_truncated = truncate_utf8_safe(last_content, 200);
    format!(
        "Conversation segment ({turn_count} turns). Started with: {first_truncated} | Ended with: {last_truncated}"
    )
}

/// Cosine similarity, clamped to `[-1, 1]`.
///
/// Zero when either vector has no magnitude, which is the honest answer: an
/// all-zero embedding has no direction to compare.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

/// Truncate at a char boundary, appending an ellipsis when anything was cut.
///
/// Counts **characters**, not bytes, so a multi-byte string is never split
/// mid-codepoint.
fn truncate_utf8_safe(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => format!("{}...", &s[..byte_idx]),
        None => s.to_string(),
    }
}

#[cfg(test)]
#[path = "boundary/boundary_tests.rs"]
mod tests;
