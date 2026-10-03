//! Controller schemas for the `tools` namespace.
//!
//! Exposes a small allowlist of tool-like operations to the Tauri shell
//! over JSON-RPC. The Tauri host needs these so the onboarding flow can
//! drive Composio + Parallel-backed web search itself (orchestration in
//! the renderer; external calls still go through the core's auth / proxy
//! layer). Anything **not** in this file remains agent-only.

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;

mod apify;
mod composio;
mod linkedin;
mod registry;
mod web_search;

pub use registry::{all_controller_schemas, all_registered_controllers};

#[cfg(test)]
use web_search::optional_string_array;
