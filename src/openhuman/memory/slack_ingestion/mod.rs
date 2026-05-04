//! Slack → memory-tree ingestion support.
//!
//! This module is the "Slack-specific plumbing the memory tree needs":
//!
//! - [`bucketer`] — 6-hour UTC-aligned window grouping + grace-period
//!   closed-bucket extraction (`split_closed`) + stable `source_id`
//!   generation.
//! - [`ops`] — bucket → [`memory::tree::canonicalize::chat::ChatBatch`]
//!   conversion + wrapper around
//!   [`memory::tree::ingest::ingest_chat`] so the caller just says
//!   "ingest this bucket for this channel".
//! - [`types`] — `SlackMessage`, `SlackChannel`, `Bucket`. Used by
//!   both the Composio-backed [`crate::openhuman::composio::providers::slack`]
//!   provider and the RPC/observability surfaces below.
//! - [`rpc`] / [`schemas`] — JSON-RPC surface for manually triggering a
//!   Slack sync (`openhuman.slack_memory_sync_trigger`) + inspecting
//!   per-connection state (`openhuman.slack_memory_sync_status`).
//!
//! Auth + scheduling live elsewhere:
//!
//! - OAuth is delegated to Composio (the user's Slack connection in
//!   Composio's hosted flow is what authorises the API calls).
//! - Periodic scheduling comes from `composio::periodic` — every 15
//!   minutes the scheduler fires `SlackProvider::sync()` for every
//!   active Slack connection.
//!
//! What this module does NOT contain (removed when we pivoted from
//! direct-bot-token to Composio):
//!
//! - A Slack Web API HTTP client. Calls go through `ctx.client.execute_tool()`.
//! - A custom per-channel cursor SQLite table. State is persisted via
//!   `composio::providers::sync_state::SyncState` in the memory KV store,
//!   keyed by `(toolkit="slack", connection_id)`.
//! - A long-running engine task. `composio::periodic::run_one_tick` is
//!   the scheduler.

pub mod bucketer;
pub mod ops;
pub mod rpc;
pub mod schemas;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_slack_ingestion_controller_schemas,
    all_registered_controllers as all_slack_ingestion_registered_controllers,
};
