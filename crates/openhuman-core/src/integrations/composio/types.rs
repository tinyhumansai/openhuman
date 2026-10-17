//! Domain types for the Composio integration.
//!
//! The definitions live in **`tinyconnectors_bus`**, the connector module's wire
//! contract, and are re-exported here so every existing
//! `integrations::composio::types::…` path keeps resolving and keeps naming the
//! same types.
//!
//! They were previously reached through `tinymemory_api::host::composio`, which
//! held them only because two crates needed to name them and there was nowhere
//! else both could. That is no longer true: the crate that *produces* these
//! shapes owns them, and memory takes them — or, after the sync migration,
//! stops needing them at all.
//!
//! **Their serde form is a wire contract**, not an implementation detail: it
//! mirrors the backend's response envelopes under
//! `/agent-integrations/composio/*`, and a field renamed on one side fails at
//! runtime with a decode error rather than at compile time. The contract crate
//! pins every one of them in a test for that reason.

pub use tinyconnectors_bus::composio::*;
