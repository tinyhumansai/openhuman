//! Phase 3 — Bounded cross-thread STM recall.
//!
//! Assembles a bounded, recency-weighted context block from two arms:
//!
//! - **Arm 1** — FTS5 over not-yet-compacted recent episodic entries from
//!   OTHER sessions. Reuses [`crate::openhuman::memory::store::fts5::episodic_cross_session_search`].
//!   When no user query is available (preemptive/session-start case), falls back to
//!   a recency selection of recent non-current-session episodic turns.
//!
//! - **Arm 2** — Brute-force cosine nearest-neighbour over `segment_embeddings`
//!   (per-model table from Phase 0+1). Single-user scale: no ANN index, no new
//!   deps. Loads candidate vectors, computes cosine, top-k. Filters by
//!   `model_signature` and excludes the current `session_id`.
//!
//! **Merge → dedup → bound:**
//! - Dedup: if a segment recap and its raw episodic rows (within
//!   `start_episodic_id..=end_episodic_id`) both appear, prefer the recap and
//!   drop the overlapping episodic hits.
//! - Recency-weight: scored by `updated_at` timestamp proximity.
//! - Hard-cap: tunable token budget ([`TOKEN_BUDGET`]) and top-k bounds
//!   ([`MAX_SEGMENT_RECAPS`] + [`MAX_EPISODIC_TURNS`]).
//!
//! ## Tunable consts (all in this file, all documented)
//! - [`RECENCY_WINDOW_DAYS`] — how many days back to search (STM/LTM boundary)
//! - [`RECENCY_WINDOW_MAX_SEGMENTS`] — max segments to load for vector search
//! - [`COSINE_GATE`] — minimum similarity for Arm 2 (medium gate)
//! - [`MAX_SEGMENT_RECAPS`] — top-k segment recaps to include
//! - [`MAX_EPISODIC_TURNS`] — max raw episodic turns to include
//! - [`TOKEN_BUDGET`] — hard token budget (chars / 4 approx)
//! - [`FTS5_LIMIT`] — how many FTS5 candidates to fetch before gating
//!
//! ## Scope boundary
//! Does NOT traverse `tree::*` (`SummaryNode`, `memory_tree_*`). The memory
//! tree is LTM; this module is strictly STM (recent episodic + segment layer).

pub mod recall;
pub mod tool;

pub use recall::{stm_recall, StmRecallBlock, StmRecallOpts};

// ─────────────────────────────────────────────────────────────────────────────
// Tunable constants — the STM/LTM knobs. Moved to `constants.rs` per the
// repo's "light mod.rs" rule; re-exported so `super::CONST` keeps resolving.
// ─────────────────────────────────────────────────────────────────────────────

mod constants;
pub use constants::*;
