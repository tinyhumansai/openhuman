//! Learning candidate buffer — Phase 1 of issue #566.
//!
//! Defines the taxonomy types ([`FacetClass`], [`CueFamily`], [`EvidenceRef`]),
//! the unit-of-work [`LearningCandidate`], and a thread-safe ring-buffer
//! [`Buffer`] that collects candidates emitted by producers (Phase 2) before
//! they are consumed by the stability detector (Phase 3).
//!
//! The buffer is bounded: when full it evicts the oldest entry (FIFO overflow).
//! A global singleton is exposed via [`global()`]; individual tests may
//! construct their own [`Buffer`] with `Buffer::new(capacity)`.

use std::collections::VecDeque;
use std::sync::OnceLock;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

// ── Taxonomy ────────────────────────────────────────────────────────────────

/// Six-class taxonomy of what the cache can hold.
///
/// Keys are stored with a class prefix, e.g. `style/verbosity` or
/// `tooling/package_manager`. The class determines the half-life and
/// class budget used by the stability detector (Phase 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacetClass {
    /// Communication style preferences — verbosity, formality, code format.
    Style,
    /// Stable biographical facts — timezone, name, language, role.
    Identity,
    /// Developer toolchain preferences — package manager, editor, OS, language.
    Tooling,
    /// Hard user vetoes — things the user has explicitly rejected or forbidden.
    Veto,
    /// Active user goals or ongoing projects.
    Goal,
    /// Preferred communication channel or platform.
    Channel,
}

/// How a candidate signal was produced — determines the weight multiplier
/// applied in the stability formula.
///
/// Higher-weight families contribute more strongly per evidence item.
/// The weights here are the canonical values from the Phase 1 plan:
/// `Explicit=1.0`, `Structural=0.9`, `Behavioral=0.7`, `Recurrence=0.6`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueFamily {
    /// Direct declaration of intent by the user (highest weight — 1.0).
    ///
    /// Examples: "I prefer pnpm", "my timezone is PST", "always use terse replies".
    Explicit,
    /// Inferred from structured file or provider metadata (weight 0.9).
    ///
    /// Examples: `package.json#packageManager`, Gmail display name, Slack workspace.
    Structural,
    /// Inferred by heuristics or LLM from observed behaviour (weight 0.7).
    ///
    /// Examples: rolling edit-window ratio, correction-repeat signal, reflection hook output.
    Behavioral,
    /// Materialized from recurrence statistics in the memory tree (weight 0.6).
    ///
    /// Examples: tree-topic hotness, source_weight per channel.
    Recurrence,
}

impl CueFamily {
    /// Weight multiplier for this cue family in the stability formula.
    ///
    /// Phase 1 canonical values (matches the plan):
    /// `Explicit=1.0`, `Structural=0.9`, `Behavioral=0.7`, `Recurrence=0.6`.
    pub fn weight(self) -> f64 {
        match self {
            CueFamily::Explicit => 1.0,
            CueFamily::Structural => 0.9,
            CueFamily::Behavioral => 0.7,
            CueFamily::Recurrence => 0.6,
        }
    }
}

// ── Evidence reference ───────────────────────────────────────────────────────

/// A typed pointer back into the memory substrate from which a candidate was
/// derived. Used for provenance tracking, citation, and the `evidence_ids`
/// column in `user_profile_facets` (Phase 3+).
///
/// Serialised with a `"type"` discriminator in snake_case so the JSON is
/// human-readable: `{"type":"episodic","episodic_id":42}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceRef {
    /// A single row in `episodic_log`.
    Episodic { episodic_id: i64 },
    /// A contiguous window of rows in `episodic_log`.
    EpisodicWindow { from_id: i64, to_id: i64 },
    /// A row in the tree-source summary table.
    SourceSummary { summary_id: String },
    /// A node in `tree_topic`.
    TreeTopic { topic_id: String },
    /// A chunk in `vector_chunks` associated with a document source.
    DocumentChunk { source_id: String, chunk_id: String },
    /// A specific message in an email source.
    EmailMessage {
        source_id: String,
        message_id: String,
    },
    /// A field value from a connected provider (Composio toolkit).
    Provider {
        toolkit: String,
        connection_id: String,
        field: String,
    },
    /// A tool call record within an episodic entry.
    ToolCall { tool_name: String, episodic_id: i64 },
    /// A per-window weight from `tree_source`.
    TreeSourceWeight { window_label: String },
}

// ── Learning candidate ───────────────────────────────────────────────────────

/// A single unit of learning evidence emitted by a producer and queued in the
/// [`Buffer`].
///
/// Each candidate asserts a specific `(class, key, value)` triple alongside
/// the evidence that backs it. The stability detector (Phase 3) aggregates
/// competing candidates for the same `(class, key)` pair and resolves them
/// into a single cache entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningCandidate {
    /// Which facet class this evidence touches.
    pub class: FacetClass,
    /// Canonical slug key within the class, e.g. `"verbosity"`, `"package_manager"`.
    ///
    /// Convention: `snake_case`, lowercase, no class prefix (the class carries that).
    pub key: String,
    /// Canonical value string, e.g. `"terse"`, `"pnpm"`, `"UTC+5:30"`.
    pub value: String,
    /// How this candidate was produced.
    pub cue_family: CueFamily,
    /// Pointer to the backing evidence in the memory substrate.
    pub evidence: EvidenceRef,
    /// Source-provided confidence hint, `0.0..=1.0`.
    ///
    /// This is an initial hint; the stability detector will reweight it using
    /// the cue-family weight and recency decay.
    pub initial_confidence: f64,
    /// When this candidate was observed, as seconds since the Unix epoch.
    pub observed_at: f64,
}

// ── Buffer ───────────────────────────────────────────────────────────────────

/// Thread-safe, bounded ring-buffer of [`LearningCandidate`] items.
///
/// Backed by a `parking_lot::Mutex<VecDeque<LearningCandidate>>`. When full
/// the oldest entry is evicted to make room (FIFO overflow). This keeps
/// memory bounded and naturally prioritises recent evidence.
///
/// The global singleton has a default capacity of 1024. Tests should
/// construct their own buffer via [`Buffer::new`].
pub struct Buffer {
    inner: Mutex<VecDeque<LearningCandidate>>,
    capacity: usize,
}

impl Buffer {
    /// Create a new buffer with the given capacity.
    ///
    /// `capacity` must be ≥ 1. A capacity of zero would make every `push`
    /// a no-op; callers should use a non-zero value.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            inner: Mutex::new(VecDeque::with_capacity(cap)),
            capacity: cap,
        }
    }

    /// Push a candidate onto the buffer.
    ///
    /// If the buffer is already at capacity, the oldest entry is evicted first
    /// (FIFO overflow). This ensures the buffer always reflects the most recent
    /// evidence.
    pub fn push(&self, candidate: LearningCandidate) {
        let mut guard = self.inner.lock();
        if guard.len() >= self.capacity {
            guard.pop_front(); // evict oldest
        }
        guard.push_back(candidate);
    }

    /// Drain all candidates from the buffer and return them in FIFO order.
    ///
    /// After this call the buffer is empty.
    pub fn drain(&self) -> Vec<LearningCandidate> {
        let mut guard = self.inner.lock();
        guard.drain(..).collect()
    }

    /// Clone all candidates without removing them.
    ///
    /// Useful for inspection or debugging.
    pub fn peek(&self) -> Vec<LearningCandidate> {
        let guard = self.inner.lock();
        guard.iter().cloned().collect()
    }

    /// Current number of candidates in the buffer.
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// Returns `true` when the buffer holds no candidates.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Maximum number of candidates the buffer will hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

// ── Global singleton ─────────────────────────────────────────────────────────

static GLOBAL_BUFFER: OnceLock<Buffer> = OnceLock::new();

/// Return the global [`Buffer`] singleton.
///
/// Initialised on first call with a default capacity of 1024. All producers
/// push into this buffer; the stability detector drains it.
pub fn global() -> &'static Buffer {
    GLOBAL_BUFFER.get_or_init(|| Buffer::new(1024))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    fn make_candidate(value: &str) -> LearningCandidate {
        LearningCandidate {
            class: FacetClass::Style,
            key: "verbosity".into(),
            value: value.into(),
            cue_family: CueFamily::Explicit,
            evidence: EvidenceRef::Episodic { episodic_id: 1 },
            initial_confidence: 0.8,
            observed_at: now_secs(),
        }
    }

    #[test]
    fn push_then_drain_preserves_fifo_order() {
        let buf = Buffer::new(10);
        buf.push(make_candidate("a"));
        buf.push(make_candidate("b"));
        buf.push(make_candidate("c"));

        let drained = buf.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].value, "a");
        assert_eq!(drained[1].value, "b");
        assert_eq!(drained[2].value, "c");
    }

    #[test]
    fn drain_empties_the_buffer() {
        let buf = Buffer::new(10);
        buf.push(make_candidate("x"));
        buf.push(make_candidate("y"));
        assert_eq!(buf.len(), 2);

        let _ = buf.drain();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn bounded_capacity_evicts_oldest() {
        let buf = Buffer::new(3);
        buf.push(make_candidate("first"));
        buf.push(make_candidate("second"));
        buf.push(make_candidate("third"));
        // Buffer is full — next push evicts "first"
        buf.push(make_candidate("fourth"));

        assert_eq!(buf.len(), 3);
        let items = buf.drain();
        assert_eq!(items[0].value, "second");
        assert_eq!(items[1].value, "third");
        assert_eq!(items[2].value, "fourth");
    }

    #[test]
    fn peek_does_not_remove() {
        let buf = Buffer::new(10);
        buf.push(make_candidate("p"));
        buf.push(make_candidate("q"));

        let peeked = buf.peek();
        assert_eq!(peeked.len(), 2);
        // Buffer still holds the items
        assert_eq!(buf.len(), 2);

        let drained = buf.drain();
        assert_eq!(drained[0].value, "p");
        assert_eq!(drained[1].value, "q");
    }

    #[test]
    fn cue_family_weight_values() {
        assert_eq!(CueFamily::Explicit.weight(), 1.0);
        assert_eq!(CueFamily::Structural.weight(), 0.9);
        assert_eq!(CueFamily::Behavioral.weight(), 0.7);
        assert_eq!(CueFamily::Recurrence.weight(), 0.6);
    }

    #[test]
    fn roundtrip_serde_evidence_ref() {
        let cases: Vec<EvidenceRef> = vec![
            EvidenceRef::Episodic { episodic_id: 42 },
            EvidenceRef::EpisodicWindow {
                from_id: 10,
                to_id: 20,
            },
            EvidenceRef::SourceSummary {
                summary_id: "sum-abc".into(),
            },
            EvidenceRef::TreeTopic {
                topic_id: "topic-xyz".into(),
            },
            EvidenceRef::DocumentChunk {
                source_id: "notion:page1".into(),
                chunk_id: "chunk-001".into(),
            },
            EvidenceRef::EmailMessage {
                source_id: "gmail:user@example.com".into(),
                message_id: "<abc123@mail.gmail.com>".into(),
            },
            EvidenceRef::Provider {
                toolkit: "gmail".into(),
                connection_id: "conn-1".into(),
                field: "display_name".into(),
            },
            EvidenceRef::ToolCall {
                tool_name: "write_file".into(),
                episodic_id: 99,
            },
            EvidenceRef::TreeSourceWeight {
                window_label: "2026-W18".into(),
            },
        ];

        for ev in &cases {
            let json = serde_json::to_string(ev).expect("serialize failed");
            let back: EvidenceRef = serde_json::from_str(&json).expect("deserialize failed");
            assert_eq!(ev, &back, "round-trip failed for variant: {json}");
        }
    }

    #[test]
    fn global_returns_same_instance_across_calls() {
        let a = global() as *const Buffer;
        let b = global() as *const Buffer;
        assert_eq!(a, b, "global() must return the same static instance");
    }
}
