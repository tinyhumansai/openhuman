//! Read-only command-center projection over the durable run ledger.
//!
//! [`list_agent_work`] fetches recent background agent runs from
//! `session_db::run_ledger` and projects them into a [`CommandCenterView`]
//! grouped by normalized [`AgentWorkBucket`]. The projection is split so the
//! pure grouping logic ([`build_view`]) is unit-testable without a database,
//! while [`list_agent_work`] owns the one ledger read.

use anyhow::Result;

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::{
    list_agent_runs, AgentRun, AgentRunListRequest, AgentRunStatus,
};

use super::types::{AgentWorkBucket, AgentWorkRow, CommandCenterGroup, CommandCenterView};

/// Default number of recent runs scanned for the command center.
const DEFAULT_LIMIT: u32 = 200;
/// Hard ceiling, mirroring the ledger's own `list_agent_runs` cap.
const MAX_LIMIT: u32 = 500;

/// Map a fine-grained ledger status to its command-center bucket.
///
/// Exhaustive on [`AgentRunStatus`] so a new ledger status variant fails to
/// compile here until its bucket is decided.
pub fn bucket_for(status: AgentRunStatus) -> AgentWorkBucket {
    match status {
        AgentRunStatus::AwaitingUser => AgentWorkBucket::NeedsInput,
        AgentRunStatus::Pending | AgentRunStatus::Running | AgentRunStatus::Paused => {
            AgentWorkBucket::Working
        }
        AgentRunStatus::Completed => AgentWorkBucket::Completed,
        AgentRunStatus::Failed => AgentWorkBucket::Failed,
        AgentRunStatus::Cancelled | AgentRunStatus::Interrupted => AgentWorkBucket::Stopped,
    }
}

/// List recent background agent work, grouped by command-center bucket.
///
/// Reads at most `limit` (default 200, capped 500) most-recently-updated runs
/// across every parent thread and projects them. Read-only: no ledger writes.
pub fn list_agent_work(config: &Config, limit: Option<u32>) -> Result<CommandCenterView> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    log::debug!(
        target: "command_center",
        "[command_center] list_agent_work.entry limit={limit}"
    );
    let request = AgentRunListRequest {
        status: None,
        kind: None,
        parent_run_id: None,
        parent_thread_id: None,
        limit: Some(limit),
        offset: None,
    };
    let response = list_agent_runs(config, &request)?;
    let view = build_view(response.runs);
    log::debug!(
        target: "command_center",
        "[command_center] list_agent_work.done total={}",
        view.total
    );
    Ok(view)
}

/// Project + group a set of ledger runs into the command-center view.
///
/// Pure: input order is preserved within each bucket, so callers that pass
/// runs already ordered most-recently-updated-first (as `list_agent_runs`
/// does) get recent-first rows per group. All five buckets are always present.
pub fn build_view(runs: Vec<AgentRun>) -> CommandCenterView {
    let rows: Vec<AgentWorkRow> = runs.into_iter().map(project_row).collect();
    let total = rows.len();
    let groups = AgentWorkBucket::ALL
        .iter()
        .map(|&bucket| {
            let bucket_rows: Vec<AgentWorkRow> = rows
                .iter()
                .filter(|r| r.bucket == bucket)
                .cloned()
                .collect();
            CommandCenterGroup {
                bucket,
                count: bucket_rows.len(),
                rows: bucket_rows,
            }
        })
        .collect();
    CommandCenterView { groups, total }
}

/// Project one ledger run into a lean command-center row.
///
/// `pub(super)` so the control verbs ([`super::control`]) can re-project a run
/// after a durable status transition without duplicating the mapping.
pub(super) fn project_row(run: AgentRun) -> AgentWorkRow {
    let display_name = run.agent_id.as_deref().and_then(resolve_display_name);
    let telemetry = run.telemetry;
    AgentWorkRow {
        run_id: run.id,
        kind: run.kind.as_str().to_string(),
        agent_id: run.agent_id,
        display_name,
        bucket: bucket_for(run.status),
        status: run.status.as_str().to_string(),
        parent_thread_id: run.parent_thread_id,
        worker_thread_id: run.worker_thread_id,
        summary: run.summary,
        error: run.error,
        started_at: run.started_at.to_rfc3339(),
        updated_at: run.updated_at.to_rfc3339(),
        elapsed_ms: telemetry.as_ref().and_then(|t| t.elapsed_ms),
        input_tokens: telemetry.as_ref().map(|t| t.input_tokens).unwrap_or(0),
        output_tokens: telemetry.as_ref().map(|t| t.output_tokens).unwrap_or(0),
        cost_usd: telemetry.as_ref().map(|t| t.cost_usd).unwrap_or(0.0),
        tool_count: telemetry.as_ref().map(|t| t.tool_count).unwrap_or(0),
    }
}

/// Resolve an agent id to its registry display name, if the registry is up and
/// the agent is known. Returns `None` otherwise (e.g. custom/removed agents).
fn resolve_display_name(agent_id: &str) -> Option<String> {
    AgentDefinitionRegistry::global()
        .and_then(|registry| registry.get(agent_id))
        .map(|definition| definition.display_name().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn run_with(id: &str, status: AgentRunStatus, updated_secs: i64) -> AgentRun {
        AgentRun {
            id: id.to_string(),
            kind: crate::openhuman::session_db::run_ledger::AgentRunKind::Subagent,
            parent_run_id: None,
            parent_thread_id: Some("thread-1".to_string()),
            agent_id: Some("researcher".to_string()),
            status,
            prompt_ref: None,
            worker_thread_id: None,
            task_board_id: None,
            task_card_id: None,
            checkpoint_path: None,
            checkpoint: None,
            summary: None,
            error: None,
            metadata: json!({}),
            telemetry: None,
            started_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_700_000_000 + updated_secs, 0).unwrap(),
            completed_at: None,
        }
    }

    #[test]
    fn bucket_for_maps_every_status_to_its_group() {
        assert_eq!(
            bucket_for(AgentRunStatus::AwaitingUser),
            AgentWorkBucket::NeedsInput
        );
        assert_eq!(
            bucket_for(AgentRunStatus::Pending),
            AgentWorkBucket::Working
        );
        assert_eq!(
            bucket_for(AgentRunStatus::Running),
            AgentWorkBucket::Working
        );
        assert_eq!(bucket_for(AgentRunStatus::Paused), AgentWorkBucket::Working);
        assert_eq!(
            bucket_for(AgentRunStatus::Completed),
            AgentWorkBucket::Completed
        );
        assert_eq!(bucket_for(AgentRunStatus::Failed), AgentWorkBucket::Failed);
        assert_eq!(
            bucket_for(AgentRunStatus::Cancelled),
            AgentWorkBucket::Stopped
        );
        assert_eq!(
            bucket_for(AgentRunStatus::Interrupted),
            AgentWorkBucket::Stopped
        );
    }

    #[test]
    fn build_view_always_emits_five_buckets_in_display_order() {
        let view = build_view(vec![]);
        assert_eq!(view.total, 0);
        let order: Vec<AgentWorkBucket> = view.groups.iter().map(|g| g.bucket).collect();
        assert_eq!(order, AgentWorkBucket::ALL.to_vec());
        assert!(view
            .groups
            .iter()
            .all(|g| g.rows.is_empty() && g.count == 0));
    }

    #[test]
    fn build_view_groups_runs_into_correct_buckets() {
        let runs = vec![
            run_with("a", AgentRunStatus::Running, 1),
            run_with("b", AgentRunStatus::AwaitingUser, 2),
            run_with("c", AgentRunStatus::Completed, 3),
            run_with("d", AgentRunStatus::Failed, 4),
            run_with("e", AgentRunStatus::Cancelled, 5),
            run_with("f", AgentRunStatus::Pending, 6),
        ];
        let view = build_view(runs);
        assert_eq!(view.total, 6);

        let group = |bucket: AgentWorkBucket| {
            view.groups
                .iter()
                .find(|g| g.bucket == bucket)
                .expect("bucket present")
        };
        assert_eq!(group(AgentWorkBucket::NeedsInput).count, 1);
        assert_eq!(group(AgentWorkBucket::Working).count, 2); // running + pending
        assert_eq!(group(AgentWorkBucket::Completed).count, 1);
        assert_eq!(group(AgentWorkBucket::Failed).count, 1);
        assert_eq!(group(AgentWorkBucket::Stopped).count, 1);
    }

    #[test]
    fn build_view_preserves_input_order_within_a_bucket() {
        // Caller passes recent-first; projection must not reorder.
        let runs = vec![
            run_with("newest", AgentRunStatus::Running, 30),
            run_with("middle", AgentRunStatus::Running, 20),
            run_with("oldest", AgentRunStatus::Running, 10),
        ];
        let view = build_view(runs);
        let working = view
            .groups
            .iter()
            .find(|g| g.bucket == AgentWorkBucket::Working)
            .unwrap();
        let ids: Vec<&str> = working.rows.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(ids, vec!["newest", "middle", "oldest"]);
    }

    #[test]
    fn project_row_defaults_telemetry_to_zero_when_absent() {
        let view = build_view(vec![run_with("x", AgentRunStatus::Completed, 1)]);
        let row = view
            .groups
            .iter()
            .flat_map(|g| &g.rows)
            .find(|r| r.run_id == "x")
            .unwrap();
        assert_eq!(row.input_tokens, 0);
        assert_eq!(row.output_tokens, 0);
        assert_eq!(row.cost_usd, 0.0);
        assert_eq!(row.tool_count, 0);
        assert_eq!(row.elapsed_ms, None);
        assert_eq!(row.status, "completed");
        assert_eq!(row.kind, "subagent");
    }
}
