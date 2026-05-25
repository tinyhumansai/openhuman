//! Memory sync pipelines.
//!
//! One top-level module hosting every "pull data from upstream → land it
//! in memory_store" pipeline, organised by the kind of upstream it talks
//! to. Three kinds today:
//!
//! - [`composio`] — Composio managed connectors (Gmail, Slack, GitHub,
//!   Notion, Linear, ClickUp, …). Pulls via the Composio Edge API.
//! - [`workspace`] — Local workspace connectors (filesystem vault sync,
//!   local-only ingest, agent-experience capture from the harness).
//! - [`mcp`] — Third-party MCP servers. Pulls via the MCP protocol over
//!   stdio/SSE.
//!
//! All three implement the [`SyncPipeline`] trait so the orchestrator
//! (`memory::jobs`) can drive them uniformly: `init` → `tick` → repeat.
//!
//! ## Layer rules
//!
//! - Sync writes into `memory_store` only — never directly into trees,
//!   never directly into unified. The ingest pipeline in
//!   `memory::ingest_pipeline` is the seam.
//! - One pipeline per upstream service. Composio's GitHub and MCP's
//!   GitHub are distinct pipelines because they hit different surfaces
//!   with different cadence and auth.
//! - Pipeline modules own their own types, their own state, and their
//!   own retry/backoff policy. The trait gives the orchestrator a
//!   single shape to call; everything else stays local.

pub mod canonicalize;
pub mod composio;
pub mod mcp;
pub mod sync_status;
pub mod traits;
pub mod workspace;

pub use traits::{SyncOutcome, SyncPipeline, SyncPipelineKind};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexports_sync_pipeline_kind_labels() {
        assert_eq!(SyncPipelineKind::Composio.as_str(), "composio");
        assert_eq!(SyncPipelineKind::Workspace.as_str(), "workspace");
        assert_eq!(SyncPipelineKind::Mcp.as_str(), "mcp");
    }
}
