//! Shared sync-pipeline trait.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;

/// The three flavors of sync pipeline. Knowing the kind at the orchestrator
/// is useful for surfaces like status dashboards and rate-limit budgeting.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPipelineKind {
    Composio,
    Workspace,
    Mcp,
}

impl SyncPipelineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncPipelineKind::Composio => "composio",
            SyncPipelineKind::Workspace => "workspace",
            SyncPipelineKind::Mcp => "mcp",
        }
    }
}

/// Result of one sync tick — minimal enough that every pipeline can fill
/// it in. Detailed per-pipeline progress lives behind the pipeline's own
/// status surface.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SyncOutcome {
    /// How many upstream records were ingested into memory_store during
    /// this tick. May be 0 when nothing new arrived.
    pub records_ingested: u32,
    /// `true` when the pipeline thinks there is more to fetch and the
    /// orchestrator should tick again soon.
    pub more_pending: bool,
    /// Free-form note for logs / status UIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Contract every sync pipeline implements. Lifecycle: `init` exactly
/// once when the pipeline first comes up, then `tick` on a cadence the
/// orchestrator picks.
#[async_trait]
pub trait SyncPipeline: Send + Sync {
    /// Stable identifier for the pipeline — e.g. `"composio:gmail"`,
    /// `"workspace:vault"`, `"mcp:filesystem"`. Used as the key in
    /// status surfaces and the job-queue.
    fn id(&self) -> &str;

    /// Which kind of pipeline this is.
    fn kind(&self) -> SyncPipelineKind;

    /// Cold-start work. Idempotent — the orchestrator may call it on
    /// every process boot.
    async fn init(&self, config: &Config) -> anyhow::Result<()>;

    /// Pull one batch from upstream and land it in memory_store. Pipeline
    /// owns its own pagination / cursor state.
    async fn tick(&self, config: &Config) -> anyhow::Result<SyncOutcome>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_pipeline_kind_as_str_matches_serde_names() {
        let cases = [
            (SyncPipelineKind::Composio, "composio"),
            (SyncPipelineKind::Workspace, "workspace"),
            (SyncPipelineKind::Mcp, "mcp"),
        ];
        for (kind, label) in cases {
            assert_eq!(kind.as_str(), label);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{label}\""));
            let decoded: SyncPipelineKind = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, kind);
        }
    }

    #[test]
    fn sync_outcome_default_is_empty_and_not_pending() {
        let outcome = SyncOutcome::default();
        assert_eq!(outcome.records_ingested, 0);
        assert!(!outcome.more_pending);
        assert!(outcome.note.is_none());
    }
}
