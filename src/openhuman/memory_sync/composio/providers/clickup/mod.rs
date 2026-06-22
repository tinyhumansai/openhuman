//! ClickUp Composio provider — incremental Memory Tree ingest for
//! tasks owned by (or assigned to) the connected user.
//!
//! Mirrors the [`crate::openhuman::memory_sync::composio::providers::notion`] layout
//! so anyone familiar with Notion/Slack ingestion can read this without
//! re-learning a new shape:
//!
//! - `provider.rs` — `impl ComposioProvider for ClickUpProvider`
//! - `sync.rs`     — payload-shape helpers (results extraction, title)
//! - `ingest.rs`   — memory_tree document ingest (issue #2885)
//! - `tools.rs`    — `CLICKUP_CURATED` whitelist of Composio actions
//! - `tests.rs`    — unit tests for the helpers + trait metadata
//!
//! Issue: #2288 (introduction); #2885 (memory_tree migration).

mod ingest;
mod provider;
mod source;
mod sync;
#[cfg(test)]
mod tests;
pub mod tools;

pub use provider::ClickUpProvider;
pub use tools::CLICKUP_CURATED;
