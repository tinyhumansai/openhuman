//! Business logic for the `subconscious_triggers` RPC surface.

use serde::{Deserialize, Serialize};

use crate::openhuman::subconscious::{ORCHESTRATOR_THREAD_ID, USER_THREAD_ID};

use super::runtime::global as orchestrator_global;

/// Snapshot of the event-driven trigger pipeline for diagnostics / UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerStatus {
    /// Whether the trigger pipeline is enabled in config.
    pub triggers_enabled: bool,
    /// Effective subconscious mode string.
    pub mode: String,
    /// Per-hour promotion cap.
    pub max_promotions_per_hour: u32,
    /// Whether the orchestrator runtime has been bootstrapped this session.
    pub orchestrator_running: bool,
    /// Pending promoted triggers awaiting the session loop, if running.
    pub queue_depth: Option<usize>,
    /// Reserved internal reasoning thread id.
    pub orchestrator_thread_id: String,
    /// Reserved user-facing thread id.
    pub user_thread_id: String,
}

/// Build the current trigger-pipeline status from config + runtime state.
pub async fn build_status() -> Result<TriggerStatus, String> {
    let config = crate::openhuman::config::load_config_with_timeout().await?;
    let hb = &config.heartbeat;
    let orchestrator = orchestrator_global();

    Ok(TriggerStatus {
        triggers_enabled: hb.triggers_enabled,
        mode: hb.effective_subconscious_mode().as_str().to_string(),
        max_promotions_per_hour: hb.max_promotions_per_hour,
        orchestrator_running: orchestrator.is_some(),
        queue_depth: orchestrator.as_ref().map(|o| o.queue_depth()),
        orchestrator_thread_id: ORCHESTRATOR_THREAD_ID.to_string(),
        user_thread_id: USER_THREAD_ID.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_reports_reserved_thread_ids() {
        let status = build_status().await.expect("status builds");
        assert_eq!(status.orchestrator_thread_id, "subconscious:orchestrator");
        assert_eq!(status.user_thread_id, "subconscious:user");
        // `orchestrator_running`/`queue_depth` reflect a process-global slot that
        // another test in this binary may have initialized, so assert only the
        // invariant between them rather than a fixed value: a running
        // orchestrator reports a queue depth; a stopped one reports none.
        assert_eq!(status.orchestrator_running, status.queue_depth.is_some());
    }
}
