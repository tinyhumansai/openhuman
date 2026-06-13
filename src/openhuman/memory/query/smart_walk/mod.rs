//! E2GraphRAG-inspired smart memory retrieval.
//!
//! Unlike the basic `walk` module which only navigates the time-based summary
//! tree, smart_walk combines multiple retrieval strategies:
//!
//! 1. **Vector search** — semantic similarity across all stored content
//! 2. **Keyword search** — pattern matching across raw content files on disk
//! 3. **Entity search** — find entities and follow relationships
//! 4. **Tree browse** — navigate wiki summary hierarchies
//! 5. **Content read** — read specific files (raw/wiki/document/episodic)
//! 6. **Source listing** — discover available sources and content types
//!
//! The walker LLM (defaulting to DeepSeek Flash) plans which strategies to
//! use, collects evidence snippets, then synthesizes a cited answer.

mod dispatch;
mod prompts;
mod runner;
mod tool;
pub mod types;

#[cfg(test)]
mod smart_walk_tests;

// ── Public re-exports ────────────────────────────────────────────────────────

pub use runner::run_smart_walk;
pub use tool::SmartMemoryWalkTool;
pub use types::{Evidence, SmartWalkOptions, SmartWalkOutcome, SmartWalkStep, SmartWalkStopReason};
