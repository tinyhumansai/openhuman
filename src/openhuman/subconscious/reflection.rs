//! Reflection primitive for the proactive subconscious layer (#623).
//!
//! Reflections are the **observation** counterpart to [`super::types::Escalation`]:
//! the LLM emits them at tick time when memory-tree signals warrant attention,
//! but unlike escalations they **never** carry an executable side effect, and
//! (unlike the original #623 design) they **never** auto-post into any
//! conversation thread. Reflections live exclusively on the Intelligence tab;
//! `proposed_action` is a free-text suggestion the user sees as a one-tap
//! button. Tapping it spawns a *new* conversation thread seeded with the
//! reflection's body + action — the existing chat thread is never bloated.
//!
//! The per-tick cap [`MAX_REFLECTIONS_PER_TICK`] guards against runaway
//! emission. Excess reflections are dropped at debug log level.

use serde::{Deserialize, Serialize};

use super::source_chunk::SourceChunk;

/// Hard cap on reflections persisted per subconscious tick. Excess are
/// dropped with a `debug!` log entry. Picked empirically: five is the
/// sweet spot between "useful proactive surface" and "notification spam".
pub const MAX_REFLECTIONS_PER_TICK: usize = 5;

/// One persisted observation about the user's state. Created by the
/// subconscious tick LLM, surfaced to the user only via the Intelligence
/// tab (no automatic conversation post).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reflection {
    /// Stable id (UUIDv4 string).
    pub id: String,
    /// What kind of signal triggered the reflection. See [`ReflectionKind`].
    pub kind: ReflectionKind,
    /// Human-readable observation body. Markdown-friendly.
    pub body: String,
    /// Optional one-tap action text. When present, the frontend renders an
    /// action button that drives `reflections_act`, which spawns a fresh
    /// conversation thread seeded with body + action.
    pub proposed_action: Option<String>,
    /// References to underlying signals (entity ids, summary ids, etc).
    /// Free-form opaque strings — used for provenance, not parsed.
    pub source_refs: Vec<String>,
    /// Resolved snapshot of the chunks the LLM cited via `source_refs`,
    /// captured at tick time. Powers (a) the Intelligence-tab "Sources"
    /// disclosure for transparency and (b) the orchestrator's memory-
    /// context injection into the system prompt for any chat turn in a
    /// thread spawned from this reflection. Snapshot semantics — chunks
    /// freeze at tick time even if the underlying entity/summary mutates
    /// later. See `super::source_chunk` for the resolver.
    #[serde(default)]
    pub source_chunks: Vec<SourceChunk>,
    /// Epoch seconds when persisted.
    pub created_at: f64,
    /// Epoch seconds when the user tapped the proposed action.
    pub acted_on_at: Option<f64>,
    /// Epoch seconds when the user dismissed the card.
    pub dismissed_at: Option<f64>,
}

/// Categorisation of the underlying signal. Start narrow; we can grow
/// the enum if a clear new bucket emerges from real data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionKind {
    /// Hotness score moved sharply for an entity since last tick.
    HotnessSpike,
    /// Multiple sources are converging on the same entity / topic.
    CrossSourcePattern,
    /// New global L0 daily digest worth highlighting.
    DailyDigest,
    /// A sealed summary references an item with a near-term deadline.
    DueItem,
    /// Pattern looks risky — concentration of negative signals, etc.
    Risk,
    /// Pattern looks like an opportunity worth user attention.
    Opportunity,
}

impl ReflectionKind {
    /// Stable lowercase string used for SQL persistence + UI chips.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HotnessSpike => "hotness_spike",
            Self::CrossSourcePattern => "cross_source_pattern",
            Self::DailyDigest => "daily_digest",
            Self::DueItem => "due_item",
            Self::Risk => "risk",
            Self::Opportunity => "opportunity",
        }
    }

    /// Inverse of [`Self::as_str`]. Falls back to [`Self::DailyDigest`]
    /// on unknown values — the most generic bucket.
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "hotness_spike" => Self::HotnessSpike,
            "cross_source_pattern" => Self::CrossSourcePattern,
            "due_item" => Self::DueItem,
            "risk" => Self::Risk,
            "opportunity" => Self::Opportunity,
            _ => Self::DailyDigest,
        }
    }
}

/// Compact wire shape that the LLM emits per reflection. Differs from
/// [`Reflection`] in that the LLM does not yet know its persisted `id`,
/// `created_at`, or any of the lifecycle timestamps. We hydrate those
/// on the Rust side before persistence.
///
/// Note: prior versions of this struct had a `disposition` field controlling
/// whether to post into a conversation thread. That auto-post path is gone —
/// reflections are now observation-only. If the LLM emits a `disposition`
/// field anyway (forward/backward compat), serde silently ignores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionDraft {
    pub kind: ReflectionKind,
    pub body: String,
    #[serde(default)]
    pub proposed_action: Option<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

/// Hydrate one [`ReflectionDraft`] into a persistable [`Reflection`].
/// Pure: callers pass `id`, `now`, and the resolved `source_chunks`
/// explicitly so tests are deterministic and the resolver can be mocked.
/// Production callers: see `engine::persist_and_surface_reflections`,
/// which calls `source_chunk::resolve_chunks` against the live config
/// before invoking this.
pub fn hydrate_draft(
    draft: ReflectionDraft,
    id: String,
    now: f64,
    source_chunks: Vec<SourceChunk>,
) -> Reflection {
    Reflection {
        id,
        kind: draft.kind,
        body: draft.body,
        proposed_action: draft.proposed_action,
        source_refs: draft.source_refs,
        source_chunks,
        created_at: now,
        acted_on_at: None,
        dismissed_at: None,
    }
}

/// Build a stable dedup key from the reflection's signal-identifying
/// fields. Two reflections with the same key and similar body should
/// not both persist within a tick — the second is the LLM repeating
/// itself rather than catching a meaningfully new signal.
///
/// The key is `kind + sorted source_refs + leading 80 chars of body`.
/// Body is included because `kind`+`source_refs` alone misses cases
/// where the same source is interpreted two different ways.
pub fn dedup_key(kind: ReflectionKind, source_refs: &[String], body: &str) -> String {
    // Canonicalize the refs: trim, drop empties, dedupe, sort. The LLM
    // sometimes echoes the same id twice in `source_refs` or sandwiches
    // whitespace; without this normalization those near-identical
    // reflections produce different keys and slip through the gate.
    let mut refs: Vec<String> = source_refs
        .iter()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    refs.sort();
    refs.dedup();
    // Canonicalize the body: collapse runs of whitespace into single
    // spaces and trim. Same rationale — a reflection with an extra
    // newline or double space at the start would otherwise key
    // differently from the original.
    let canonical_body: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let body_prefix: String = canonical_body.chars().take(80).collect();
    format!("{}|{}|{}", kind.as_str(), refs.join(","), body_prefix)
}

/// Apply the per-tick cap to a list of drafts, dropping the tail. Returns
/// the kept slice along with the count dropped (so the caller can log
/// it at debug level).
pub fn apply_cap(drafts: Vec<ReflectionDraft>) -> (Vec<ReflectionDraft>, usize) {
    if drafts.len() <= MAX_REFLECTIONS_PER_TICK {
        return (drafts, 0);
    }
    let dropped = drafts.len() - MAX_REFLECTIONS_PER_TICK;
    let kept = drafts.into_iter().take(MAX_REFLECTIONS_PER_TICK).collect();
    (kept, dropped)
}

#[cfg(test)]
#[path = "reflection_tests.rs"]
mod tests;
