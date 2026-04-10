//! Layered context-reduction pipeline.
//!
//! Context summarisation in openhuman is layered, not a single knob.
//! Each layer has a specific trigger and invariant:
//!
//! | Stage | File                           | When                           | Cache impact   |
//! |-------|--------------------------------|--------------------------------|----------------|
//! | 1. Tool-result budget | [`tool_result_budget`] | New tool result created  | Cache-safe     |
//! | 2. Snip / trim        | `Agent::trim_history`  | Message count > max      | Cache-safe*    |
//! | 3. Microcompact       | [`microcompact`]       | Guard ≥ 90% soft bound   | Breaks prefix  |
//! | 4. Autocompact        | `loop_/history.rs`     | Microcompact not enough  | Breaks prefix  |
//! | 5. Session memory     | [`session_memory`]     | Token/turn/tool deltas   | Async, free    |
//!
//! \* Trim only drops messages older than the most-recent stable prefix,
//! which is often outside the KV-cache anyway.
//!
//! The orchestrator is [`pipeline::ContextPipeline`], which is owned by
//! the `Agent` and called once per turn before each provider hit. Stage
//! 1 is applied inline in `Agent::execute_tool_call`, not here.
//!
//! Stage reference:
//! - [`tool_result_budget::apply_tool_result_budget`] — stage 1
//! - [`microcompact::microcompact`] — stage 3
//! - `PipelineOutcome::AutocompactionRequested` — stage 4 signal
//! - [`session_memory::SessionMemoryState`] — stage 5 state tracker

pub mod microcompact;
pub mod pipeline;
pub mod session_memory;
pub mod tool_result_budget;

pub use microcompact::{
    microcompact, MicrocompactStats, CLEARED_PLACEHOLDER, DEFAULT_KEEP_RECENT_TOOL_RESULTS,
};
pub use pipeline::{ContextPipeline, ContextPipelineConfig, PipelineOutcome};
pub use session_memory::{
    SessionMemoryConfig, SessionMemoryState, ARCHIVIST_EXTRACTION_PROMPT, DEFAULT_MIN_TOKEN_GROWTH,
    DEFAULT_MIN_TOOL_CALLS, DEFAULT_MIN_TURNS_BETWEEN,
};
pub use tool_result_budget::{
    apply_tool_result_budget, BudgetOutcome, DEFAULT_TOOL_RESULT_BUDGET_BYTES,
};
