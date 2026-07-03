//! One-time import of legacy OpenHuman sessions into TinyAgents stores.
//!
//! Implements the P1 migration from `docs/tinyagents-session-migration-design.md`
//! (issue #4249): legacy transcript JSONL (`session_raw/`, flat and
//! `DDMMYYYY` date folders) and legacy Markdown sessions are normalized into
//! TinyAgents `Store`/`AppendStore` records under
//! `{workspace}/tinyagents_store/`. Sources are never mutated; the command is
//! idempotent (global marker + per-item fingerprint ledger) and exposed as
//! `openhuman.session_import_run` (`openhuman-core session-import run`). It
//! is an explicit command, never a boot hook.

mod convert;
pub mod live;
pub mod ops;
mod scan;
mod schemas;
pub mod types;

pub use schemas::{
    all_session_import_controller_schemas, all_session_import_registered_controllers,
};
pub use types::{ImportOptions, ImportSummary};

#[cfg(test)]
mod ops_tests;
