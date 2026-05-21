//! GitHub Composio provider — incremental Memory Tree ingest for
//! GitHub issues assigned to the connected user.
//!
//! Mirrors the [`crate::openhuman::composio::providers::clickup`] and
//! [`crate::openhuman::composio::providers::linear`] layouts so anyone
//! familiar with those providers can read this without re-learning a
//! new shape:
//!
//! - `provider.rs` — `impl ComposioProvider for GitHubProvider`
//! - `sync.rs`     — payload-shape helpers (results / title / cursor / viewer)
//! - `tools.rs`    — `GITHUB_CURATED` whitelist of Composio actions
//! - `tests.rs`    — unit tests for the helpers + trait metadata
//!
//! Issue: #2408.
//!
//! Note: `tools.rs` predates this provider — when GitHub was catalog-only
//! the file already housed the curated action list. Promoting GitHub to a
//! full provider only required adding `provider.rs` / `sync.rs` / `tests.rs`
//! around it; the curated catalog (used by `catalog_for_toolkit("github")`
//! in `super::mod`) is unchanged.

mod provider;
mod sync;
#[cfg(test)]
mod tests;
pub mod tools;

pub use provider::GitHubProvider;
pub use tools::GITHUB_CURATED;
