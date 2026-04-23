//! Session manager (A4).
//!
//! Consumes `chronicle_events` produced by A3's dispatcher/parser and
//! groups them into coherent work sessions bounded by idle gaps, sustained
//! app switches, or a hard 2h cap. Session headers and 1-minute rollup
//! buckets are persisted for A6 (daily reducer) to consume.
//!
//! * `rules` — pure boundary-detection logic (idle-5m / app_switch-3m /
//!   max-2h) and minute-bucket truncation helpers.
//! * `tables` — SQLite writer for `chronicle_sessions` and
//!   `chronicle_minute_buckets`.
//! * `manager` — stateful per-process actor that reads new chronicle
//!   rows past the `"session_manager"` watermark, applies rules, and
//!   persists closed sessions atomically.
//! * `runtime` — `tokio` interval-driven ticker that wires the manager
//!   into core startup.
//!
//! Out of scope for A4: RPC surface (A6 adds what it needs); durable
//! persistence of the in-memory open session (acceptable to lose the
//! partial header on restart — no event data is lost).

pub mod manager;
pub mod rules;
pub mod runtime;
pub mod tables;

pub use manager::{SessionManager, TickResult, WATERMARK_SOURCE};
pub use runtime::{spawn, TICK_INTERVAL};
