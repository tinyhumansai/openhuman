//! Linear Composio provider — incremental Memory Tree ingest for
//! Linear issues assigned to the connected user.
//!
//! Mirrors the [`crate::openhuman::composio::providers::clickup`] and
//! [`crate::openhuman::composio::providers::notion`] layouts so anyone
//! familiar with those providers can read this without re-learning a
//! new shape:
//!
//! - `provider.rs` — `impl ComposioProvider for LinearProvider`
//! - `sync.rs`     — payload-shape helpers (results extraction, title,
//!                   cursor, user-id, workspace identifiers)
//! - `tools.rs`    — `LINEAR_CURATED` whitelist of Composio actions
//! - `tests.rs`    — unit tests for the helpers + trait metadata
//!
//! Issue: #2400.

mod provider;
mod sync;
#[cfg(test)]
mod tests;
pub mod tools;

pub use provider::LinearProvider;
pub use tools::LINEAR_CURATED;
