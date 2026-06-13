//! GitHub Composio provider — incremental Memory Tree ingest for issues and
//! pull requests involving the connected user.
//!
//! Mirrors the [`crate::openhuman::memory_sync::composio::providers::clickup`] layout so
//! anyone familiar with ClickUp/Notion ingestion can read this without
//! re-learning a new shape:
//!
//! - `provider.rs` — `impl ComposioProvider for GitHubProvider`
//! - `sync.rs`     — payload-shape helpers (result extraction, title, cursor)
//! - `tools.rs`    — `GITHUB_CURATED` whitelist of Composio actions
//! - `tests.rs`    — unit tests for the helpers + trait metadata
//!
//! Issue: #2408.

mod ingest;
mod provider;
mod sync;
#[cfg(test)]
mod tests;
pub mod tools;

pub use provider::GitHubProvider;
pub use tools::GITHUB_CURATED;
