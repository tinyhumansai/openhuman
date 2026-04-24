//! Agent-facing tool surface for the `life_capture` PersonalIndex.
//!
//! Ingest paths (iMessage scanner, Composio Gmail/Calendar bridge) write
//! into the same SQLite + sqlite-vec index. This tool exposes the read
//! side to the LLM so it can answer "what did X say about Y" / "find the
//! meeting where we discussed Z" directly instead of only relying on the
//! small curated MEMORY.md + USER.md snapshot.

pub mod search;

pub use search::LifeCaptureSearchTool;
