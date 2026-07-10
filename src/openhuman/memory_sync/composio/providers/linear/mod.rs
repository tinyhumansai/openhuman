//! Linear Composio provider — incremental Memory Tree ingest for
//! issues assigned to the connected user.
//!
//! Issue: #2400.

mod ingest;
mod provider;
mod source;
mod sync;
#[cfg(test)]
mod tests;
pub mod tools;

pub use provider::LinearProvider;
pub use tools::LINEAR_CURATED;
