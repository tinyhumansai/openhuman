//! Reflection primitive for the proactive subconscious layer (#623).
//!
//! Reflections are the **observation** counterpart to [`super::types::Escalation`]:
//! the LLM emits them at tick time when memory-tree signals warrant attention,
//! but unlike escalations they **never** carry an executable side effect.
//! `proposed_action` is a free-text suggestion the user sees as a one-tap
//! button — the user (or the agent on the user's behalf) chooses whether to
//! act on it.
//!
//! Two dispositions:
//! - [`Disposition::Observe`] — quietly persisted; visible in next tick's
//!   "Recent reflections" prompt section so the LLM can decay or strengthen.
//! - [`Disposition::Notify`] — also surfaced as a message in the dedicated
//!   `system:subconscious` conversation thread.
//!
//! The per-tick cap [`MAX_REFLECTIONS_PER_TICK`] guards against runaway
//! emission. Excess reflections are dropped at debug log level.

use serde::{Deserialize, Serialize};

/// Hard cap on reflections persisted per subconscious tick. Excess are
/// dropped with a `debug!` log entry. Picked empirically: five is the
/// sweet spot between "useful proactive surface" and "notification spam".
pub const MAX_REFLECTIONS_PER_TICK: usize = 5;

/// One persisted observation about the user's state. Created by the
/// subconscious tick LLM, surfaced to the user via the Intelligence tab
/// and (for `Notify`) the `system:subconscious` conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reflection {
    /// Stable id (UUIDv4 string).
    pub id: String,
    /// What kind of signal triggered the reflection. See [`ReflectionKind`].
    pub kind: ReflectionKind,
    /// Human-readable observation body. Markdown-friendly.
    pub body: String,
    /// Whether to surface in conversation or only persist quietly.
    pub disposition: Disposition,
    /// Optional one-tap action text. When present and `Notify`, the
    /// frontend renders an action button that drives `reflections_act`.
    pub proposed_action: Option<String>,
    /// References to underlying signals (entity ids, summary ids, etc).
    /// Free-form opaque strings — used for provenance, not parsed.
    pub source_refs: Vec<String>,
    /// Epoch seconds when persisted.
    pub created_at: f64,
    /// Epoch seconds when posted into the subconscious conversation
    /// (only set for `Notify`).
    pub surfaced_at: Option<f64>,
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

/// Whether the LLM chose to surface this reflection in the
/// subconscious conversation thread (`Notify`) or keep it as a quiet
/// observation visible only in the Intelligence tab (`Observe`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Persist only — visible in "Recent reflections" next tick + on the
    /// Intelligence tab. No conversation post.
    Observe,
    /// Persist + post into the `system:subconscious` thread.
    Notify,
}

impl Disposition {
    /// Stable lowercase string used for SQL persistence + UI badges.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Notify => "notify",
        }
    }

    /// Inverse of [`Self::as_str`]. Defaults to `Observe` on unknown
    /// values so a forward-compatible UI never loses a row.
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "notify" => Self::Notify,
            _ => Self::Observe,
        }
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionDraft {
    pub kind: ReflectionKind,
    pub body: String,
    pub disposition: Disposition,
    #[serde(default)]
    pub proposed_action: Option<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

/// Hydrate one [`ReflectionDraft`] into a persistable [`Reflection`].
/// Pure: callers pass `id` and `now` explicitly so tests are
/// deterministic.
pub fn hydrate_draft(draft: ReflectionDraft, id: String, now: f64) -> Reflection {
    Reflection {
        id,
        kind: draft.kind,
        body: draft.body,
        disposition: draft.disposition,
        proposed_action: draft.proposed_action,
        source_refs: draft.source_refs,
        created_at: now,
        surfaced_at: None,
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
    let mut refs = source_refs.to_vec();
    refs.sort();
    let body_prefix: String = body.chars().take(80).collect();
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
