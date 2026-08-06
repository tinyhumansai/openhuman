//! Public types for the transcript-to-memory ingestion pipeline.

use serde::{Deserialize, Serialize};

/// Memory namespace where transcript-derived durable facts live.
///
/// Kept distinct from `learning_observations` (turn-level reflection),
/// `learning_reflections` (LLM-extracted user reflections) and
/// `working.user.*` (sync-derived profile facts) so retrieval can target
/// transcript-only memory without polluting other sources.
pub const CONVERSATION_MEMORY_NAMESPACE: &str = "conversation_memory";

/// Memory namespace for transcript-derived higher-level reflections —
/// patterns, repeated mistakes, opportunities. Surfaced through the
/// subconscious / Intelligence UI rather than the prompt context block.
pub const CONVERSATION_REFLECTIONS_NAMESPACE: &str = "conversation_reflections";

/// Importance tier — controls which memories are surfaced into a fresh
/// chat by default. Only `High` candidates make it into the prompt block;
/// `Medium` is retrievable on demand; `Low` is stored but never auto-
/// surfaced (kept for audit / debugging).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Importance {
    Low,
    Medium,
    High,
}

impl Importance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "med",
            Self::High => "high",
        }
    }
}

/// Discriminator for what a memory candidate represents. Drives the
/// human-readable prefix on the stored content and downstream filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateKind {
    Preference,
    Decision,
    Commitment,
    UnresolvedTask,
    Fact,
}

impl CandidateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Decision => "decision",
            Self::Commitment => "commitment",
            Self::UnresolvedTask => "unresolved_task",
            Self::Fact => "fact",
        }
    }
}

/// Provenance metadata attached to every persisted memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// Backend `thread_id` from the transcript meta header, if known.
    pub thread_id: Option<String>,
    /// Full transcript path (display form) — useful for debugging.
    pub transcript_path: String,
    /// Just the file basename (e.g. `1714000000_main.jsonl`) — included
    /// in the rendered content so readers don't see absolute paths.
    pub transcript_basename: String,
    /// Indices of the source messages within the transcript message
    /// array. A reflection or merged fact may cite multiple indices.
    pub message_indices: Vec<usize>,
    /// RFC-3339 timestamp of when the candidate was extracted.
    pub extracted_at: String,
}

/// A memory candidate ready to persist.
#[derive(Debug, Clone)]
pub struct MemoryCandidate {
    pub kind: CandidateKind,
    pub importance: Importance,
    pub content: String,
    pub provenance: Provenance,
}

/// A higher-level reflection extracted from a transcript window —
/// patterns, recurring themes, repeated failures, improvement signals.
#[derive(Debug, Clone)]
pub struct ConversationReflection {
    pub importance: Importance,
    pub theme: String,
    pub detail: String,
    pub provenance: Provenance,
}

/// Summary of one ingestion pass — surfaced in logs and returned to
/// callers (mainly tests) for assertion.
#[derive(Debug, Clone, Default)]
pub struct IngestionReport {
    pub processed_messages: usize,
    pub extracted: usize,
    pub stored: usize,
    pub deduped: usize,
    pub reflections_extracted: usize,
    pub reflections_stored: usize,
    pub candidates: Vec<MemoryCandidate>,
    pub reflections: Vec<ConversationReflection>,
}
