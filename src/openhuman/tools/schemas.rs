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
include!("schemas_part_01.rs");
include!("schemas_part_02.rs");
