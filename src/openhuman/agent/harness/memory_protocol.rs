//! Memory-protocol enforcement state machine (issue #4116).
//!
//! Agents are instructed to follow a **read-index → dedupe → write →
//! update-index** cycle when they mutate durable memory:
//!
//! 1. Read the memory index (`memory_recall` / a `memory_tree` query, or
//!    equivalently the `MEMORY.md` index) to check for near-duplicates *before*
//!    creating an entry.
//! 2. Write the entry (`memory_store`, `memory_forget`, or a document ingest via
//!    `memory_tree_ingest_document` / `memory_tree` with `mode: "ingest_document"`).
//! 3. Call `update_memory_md` (targeting `MEMORY.md`) afterward so the index
//!    stays in sync with the underlying store.
//!
//! The protocol was previously described to the model but never enforced, so it
//! was followed inconsistently — agents wrote entries without a dedupe read
//! (creating duplicates) and skipped `update_memory_md` (so the index drifted).
//!
//! This module is the pure, side-effect-free state machine that observes the
//! sequence of memory tool calls in a session and reports two violations:
//!
//! - **missing index read** — a write not preceded by an index read this cycle.
//! - **index drift** — a write that was never followed by `update_memory_md`
//!   (detected at the next write, and at run end via [`MemoryProtocolTracker::pending_index_update`]).
//!
//! The tinyagents seam ([`crate::openhuman::tinyagents::middleware`]) drives this
//! tracker from its `after_tool` / `after_agent` hooks and surfaces the guidance
//! back to the model as a corrective note appended to the tool result — the same
//! "structured correction surfaced to the model" pattern used for unknown-tool
//! recovery (#4118). Keeping the logic here (free of tinyagents types) lets the
//! read → dedupe → write → update-index contract be unit-tested directly.

/// Marker prefixed to every corrective note so downstream code (and tests) can
/// recognise memory-protocol guidance in a tool result.
pub const MEMORY_PROTOCOL_MARKER: &str = "[memory-protocol]";

/// Classification of a tool call for the memory protocol. Everything the model
/// can call is one of these; non-memory tools are [`MemoryOp::Other`] and never
/// affect protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOp {
    /// Reading the memory index to check for duplicates (satisfies the "read
    /// index / dedupe" step of the cycle).
    IndexRead,
    /// A durable memory mutation that should be preceded by a dedupe read and
    /// followed by an index update.
    Write,
    /// Writing the `MEMORY.md` index back into sync (the closing step).
    IndexUpdate,
    /// Any tool that is not part of the memory protocol.
    Other,
}

/// Classify a tool call into a [`MemoryOp`], keyed by name and — for the two
/// multi-purpose tools — the arguments.
///
/// Two tools are polymorphic and cannot be classified by name alone:
/// - `update_memory_md` edits either `MEMORY.md` **or** `SKILL.md`
///   (`src/openhuman/tools/impl/filesystem/update_memory_md.rs`); only a
///   `MEMORY.md` edit reconciles the memory index, so a `SKILL.md` edit is not
///   an [`MemoryOp::IndexUpdate`] and must not close the cycle.
/// - the consolidated `memory_tree` tool (`src/openhuman/memory/query/mod.rs`)
///   is a read in every `mode` except `ingest_document`, which writes a document
///   into the tree — a durable mutation.
///
/// The arguments are captured at `before_tool` time and correlated to the result
/// by call id (the tool result itself carries no arguments).
pub fn classify_memory_op(tool_name: &str, arguments: &serde_json::Value) -> MemoryOp {
    let arg_str = |key: &str| arguments.get(key).and_then(|v| v.as_str());
    match tool_name {
        // The index-sync step — but only for the MEMORY.md index. The same tool
        // can edit SKILL.md, which does not reconcile the memory index and so is
        // a no-op for this protocol.
        "update_memory_md" => match arg_str("file") {
            Some("MEMORY.md") => MemoryOp::IndexUpdate,
            _ => MemoryOp::Other,
        },
        // Durable mutations: create an entry, delete an entry, or ingest a
        // document into the memory tree (the split-out ingest tool).
        "memory_store" | "memory_forget" | "memory_tree_ingest_document" => MemoryOp::Write,
        // `remember_preference` / `save_preference` (#4458): these DO persist via
        // `Memory::store`, but into dedicated preference namespaces
        // (`pinned_preferences` / `user_pref_{general,situational}`) that are
        // surfaced by direct system-prompt injection or per-query recall — they
        // bypass the inference/stability pipeline and are NOT part of the
        // `MEMORY.md` curated wiki the archivist reconciles. So they are neither
        // an index write that needs a dedupe read nor one that must be closed by
        // `update_memory_md`; treating them as `Write` would resurrect the very
        // unsatisfiable "call update_memory_md" nag loop this issue removes.
        // Classified `Other` deliberately (explicit arm, not fall-through).
        "remember_preference" | "save_preference" => MemoryOp::Other,
        // Consolidated memory_tree tool: `ingest_document` writes; every other
        // mode is a read-only retrieval.
        "memory_tree" => match arg_str("mode") {
            Some("ingest_document") => MemoryOp::Write,
            _ => MemoryOp::IndexRead,
        },
        // Dedupe reads: recall/search over stored memory, or a read-only walk of
        // the memory tree. These let the agent check for near-duplicates before
        // writing.
        "memory_recall"
        | "memory_vector_search"
        | "memory_chunk_context"
        | "memory_hybrid_search"
        | "memory_tree_query_source"
        | "memory_tree_search_entities"
        | "memory_tree_fetch_leaves"
        | "memory_tree_drill_down"
        | "memory_tree_cover_window" => MemoryOp::IndexRead,
        _ => MemoryOp::Other,
    }
}

/// What a single observed memory op means for the protocol. Returned by
/// [`MemoryProtocolTracker::observe`] so the caller can decide whether — and
/// what — to surface back to the model.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemoryProtocolObservation {
    /// The observed op was a durable memory write.
    pub was_write: bool,
    /// A write happened without a dedupe/index read earlier in this cycle.
    pub missing_index_read: bool,
    /// A write happened while a previous write was still awaiting
    /// `update_memory_md` — the index is drifting from the store.
    pub index_drift: bool,
}

impl MemoryProtocolObservation {
    /// Whether this observation warrants a corrective note back to the model.
    /// Every write gets one (the forward "call `update_memory_md`" reminder is
    /// itself the enforcement of the closing step); reads and other ops don't.
    pub fn needs_guidance(&self) -> bool {
        self.was_write
    }

    /// Render the corrective note appended to the tool result, or `None` when no
    /// guidance is warranted. The wording escalates with the violations detected.
    pub fn guidance(&self, tool_name: &str) -> Option<String> {
        if !self.needs_guidance() {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        if self.missing_index_read {
            parts.push(format!(
                "`{tool_name}` wrote to memory without first reading the memory index to check for \
                 duplicates. Before creating entries, recall existing memory (e.g. `memory_recall`) \
                 so you don't store a near-duplicate."
            ));
        }
        if self.index_drift {
            parts.push(
                "A previous memory write was never followed by `update_memory_md`, so the MEMORY.md \
                 index is drifting from stored memory. Reconcile it now."
                    .to_string(),
            );
        }
        // The always-on closing-step reminder: keep the index in sync.
        parts.push(
            "After mutating memory, call `update_memory_md` to keep the MEMORY.md index in sync."
                .to_string(),
        );
        Some(format!("{MEMORY_PROTOCOL_MARKER} {}", parts.join(" ")))
    }
}

/// Per-session tracker of the memory-protocol cycle. One instance lives for the
/// duration of a turn/run (held by the tinyagents middleware) and observes the
/// ordered sequence of *successful* memory tool calls.
///
/// The cycle is `read → write → update-index`, and it repeats: an
/// `update_memory_md` closes a cycle and arms the next one (so the following
/// write again expects a fresh dedupe read).
#[derive(Debug, Default)]
pub struct MemoryProtocolTracker {
    /// A dedupe/index read has occurred since the last cycle reset.
    saw_index_read: bool,
    /// A write has occurred that has not yet been followed by `update_memory_md`.
    pending_index_update: bool,
}

impl MemoryProtocolTracker {
    /// Fresh tracker with no observed ops.
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe one **successful** memory op and advance the state machine,
    /// returning what it means for the protocol. Callers must only pass ops that
    /// actually succeeded — a failed `memory_store` neither creates an entry nor
    /// obliges an index update.
    pub fn observe(&mut self, op: MemoryOp) -> MemoryProtocolObservation {
        match op {
            MemoryOp::IndexRead => {
                self.saw_index_read = true;
                MemoryProtocolObservation::default()
            }
            MemoryOp::Write => {
                let obs = MemoryProtocolObservation {
                    was_write: true,
                    missing_index_read: !self.saw_index_read,
                    index_drift: self.pending_index_update,
                };
                self.pending_index_update = true;
                obs
            }
            MemoryOp::IndexUpdate => {
                // The index is back in sync; arm the next cycle so its write
                // expects a fresh dedupe read.
                self.pending_index_update = false;
                self.saw_index_read = false;
                MemoryProtocolObservation::default()
            }
            MemoryOp::Other => MemoryProtocolObservation::default(),
        }
    }

    /// Classify a tool call (name + arguments) and [`observe`](Self::observe) it
    /// in one step.
    pub fn observe_tool(
        &mut self,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> MemoryProtocolObservation {
        self.observe(classify_memory_op(tool_name, arguments))
    }

    /// Whether a memory write is still awaiting `update_memory_md`. Checked at
    /// run end to detect a write that was never followed by an index update.
    pub fn pending_index_update(&self) -> bool {
        self.pending_index_update
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Empty arguments — for the name-only tools where arguments are irrelevant.
    fn no_args() -> serde_json::Value {
        json!({})
    }

    #[test]
    fn classifies_the_memory_tool_surface() {
        let a = no_args();
        assert_eq!(classify_memory_op("memory_store", &a), MemoryOp::Write);
        assert_eq!(classify_memory_op("memory_forget", &a), MemoryOp::Write);
        assert_eq!(
            classify_memory_op("memory_tree_ingest_document", &a),
            MemoryOp::Write
        );
        assert_eq!(classify_memory_op("memory_recall", &a), MemoryOp::IndexRead);
        assert_eq!(
            classify_memory_op("memory_vector_search", &a),
            MemoryOp::IndexRead
        );
        assert_eq!(
            classify_memory_op("memory_tree_search_entities", &a),
            MemoryOp::IndexRead
        );
        assert_eq!(classify_memory_op("send_message", &a), MemoryOp::Other);
        assert_eq!(classify_memory_op("file_write", &a), MemoryOp::Other);
    }

    #[test]
    fn update_memory_md_only_closes_the_cycle_for_the_memory_index() {
        // A MEMORY.md edit reconciles the index; a SKILL.md edit does not and so
        // must not close the cycle or clear a pending write.
        assert_eq!(
            classify_memory_op("update_memory_md", &json!({ "file": "MEMORY.md" })),
            MemoryOp::IndexUpdate
        );
        assert_eq!(
            classify_memory_op("update_memory_md", &json!({ "file": "SKILL.md" })),
            MemoryOp::Other
        );

        // A SKILL.md update after a memory write leaves the index still owed.
        let mut t = MemoryProtocolTracker::new();
        t.observe_tool("memory_recall", &no_args());
        t.observe_tool("memory_store", &no_args());
        t.observe_tool("update_memory_md", &json!({ "file": "SKILL.md" }));
        assert!(
            t.pending_index_update(),
            "a SKILL.md edit must not mask the stale MEMORY.md index"
        );
    }

    #[test]
    fn consolidated_memory_tree_ingest_is_a_write() {
        // Every mode is a read except `ingest_document`, which writes.
        assert_eq!(
            classify_memory_op("memory_tree", &json!({ "mode": "ingest_document" })),
            MemoryOp::Write
        );
        assert_eq!(
            classify_memory_op("memory_tree", &json!({ "mode": "search_entities" })),
            MemoryOp::IndexRead
        );

        // An ingest via the consolidated tool obliges an index update.
        let mut t = MemoryProtocolTracker::new();
        let obs = t.observe_tool("memory_tree", &json!({ "mode": "ingest_document" }));
        assert!(obs.was_write, "ingest_document mode is a durable write");
        assert!(t.pending_index_update());
    }

    #[test]
    fn full_cycle_reports_no_violation() {
        let mut t = MemoryProtocolTracker::new();
        assert_eq!(
            t.observe_tool("memory_recall", &no_args()),
            Default::default()
        );

        let write = t.observe_tool("memory_store", &no_args());
        assert!(write.was_write);
        assert!(!write.missing_index_read, "read preceded the write");
        assert!(!write.index_drift);
        assert!(t.pending_index_update());

        // Closing the cycle clears the pending index update.
        assert_eq!(
            t.observe_tool("update_memory_md", &json!({ "file": "MEMORY.md" })),
            Default::default()
        );
        assert!(!t.pending_index_update());
    }

    #[test]
    fn write_without_index_read_is_flagged() {
        let mut t = MemoryProtocolTracker::new();
        let obs = t.observe_tool("memory_store", &no_args());
        assert!(obs.was_write);
        assert!(obs.missing_index_read, "no dedupe read preceded the write");
        assert!(obs.needs_guidance());
        let note = obs.guidance("memory_store").expect("guidance for a write");
        assert!(note.starts_with(MEMORY_PROTOCOL_MARKER));
        assert!(note.contains("without first reading the memory index"));
        assert!(note.contains("update_memory_md"));
    }

    #[test]
    fn write_not_followed_by_update_is_detected_at_next_write() {
        let mut t = MemoryProtocolTracker::new();
        t.observe_tool("memory_recall", &no_args());
        let first = t.observe_tool("memory_store", &no_args());
        assert!(!first.index_drift);
        assert!(t.pending_index_update());

        // A second write with no intervening update_memory_md: the index is
        // drifting from the store.
        let second = t.observe_tool("memory_store", &no_args());
        assert!(second.index_drift, "prior write never synced the index");
        let note = second.guidance("memory_store").unwrap();
        assert!(note.contains("drifting"));
    }

    #[test]
    fn pending_index_update_survives_until_update_at_run_end() {
        let mut t = MemoryProtocolTracker::new();
        t.observe_tool("memory_recall", &no_args());
        t.observe_tool("memory_store", &no_args());
        // Intervening non-memory tool calls don't clear the obligation.
        t.observe_tool("send_message", &no_args());
        assert!(
            t.pending_index_update(),
            "index update still owed at run end"
        );
    }

    #[test]
    fn update_arms_a_fresh_cycle_that_expects_a_new_read() {
        let mut t = MemoryProtocolTracker::new();
        t.observe_tool("memory_recall", &no_args());
        t.observe_tool("memory_store", &no_args());
        t.observe_tool("update_memory_md", &json!({ "file": "MEMORY.md" }));

        // Next cycle: a write with no fresh read is flagged again.
        let obs = t.observe_tool("memory_store", &no_args());
        assert!(
            obs.missing_index_read,
            "each cycle needs its own dedupe read"
        );
    }

    #[test]
    fn reads_and_other_ops_need_no_guidance() {
        let mut t = MemoryProtocolTracker::new();
        assert!(!t.observe_tool("memory_recall", &no_args()).needs_guidance());
        assert!(!t.observe_tool("send_message", &no_args()).needs_guidance());
        assert!(t
            .observe_tool("memory_recall", &no_args())
            .guidance("memory_recall")
            .is_none());
    }
}
