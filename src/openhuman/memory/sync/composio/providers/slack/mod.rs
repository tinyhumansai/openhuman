//! The Composio-backed Slack provider's JSON-RPC surface.
//!
//! This used to glob-import `tinymemory_core::sync::composio::providers::slack`,
//! the engine's native `SlackProvider` (its own `sync()`, `post_process`,
//! `run_backfill_via_search`, …). tinymemory v1.13.4 deleted that provider
//! along with the rest of the in-process Composio pipeline — see
//! `crate::openhuman::integrations::composio::providers` for where each piece
//! went. `rpc.rs` now reads a Slack connection through the `tinyconnectors`
//! module and hands what it returns to the bound memory driver via
//! `MemorySourceSink::accept_source_items`, the same path
//! `integrations::composio::ops::providers_ops::run_sync_pass` uses for every
//! other toolkit.
//!
//! What survives here is this host's own RPC pair —
//! `openhuman.slack_memory_sync_trigger` / `openhuman.slack_memory_sync_status`
//! — which never depended on the engine's Slack-specific parsing to begin
//! with; only on the driver seam that is now the connector module.

pub mod rpc;
pub mod schemas;

// The controller aggregators this domain's RPC surface defines.
pub use schemas::*;
