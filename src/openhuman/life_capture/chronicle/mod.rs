//! Chronicle dispatcher + parser (A3).
//!
//! Stage 0 (`dispatcher`) — receives raw focus/capture events from
//! accessibility, applies dedup and debounce. Stage 1 (`parser`) —
//! normalises the deduped event into a `ChronicleEvent` with PII-redacted
//! visible text and URL extraction (for browser-class apps only).
//!
//! Storage tables and cursor watermark live in the same SQLite database as
//! the life_capture personal index — see `migrations/0003_chronicle.sql`.
//!
//! Out of scope for A3: session bucketing (A4), daily reduction (A6),
//! entity extraction (A8), LLM calls of any kind.

pub mod dispatcher;
pub mod parser;
pub mod rpc;
pub mod schemas;
pub mod tables;

pub use schemas::{
    all_controller_schemas as all_chronicle_controller_schemas,
    all_registered_controllers as all_chronicle_registered_controllers,
};
