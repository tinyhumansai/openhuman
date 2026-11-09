//! OpenHuman-owned workflow event effects: Langfuse export and approval
//! notification. Execution, run rows, and graph observations belong to the
//! TinyFlows module; this file only consumes its typed PollRun projection.

use serde_json::json;
use tinyflows::engine::GraphObservation;
use tinyflows_bus::catalog::Workflow as Flow;
use tinyflows_bus::runs::FlowRunTrigger;

use crate::config::Config;

#[path = "langfuse_export.rs"]
mod langfuse_export;

/// Exports one module-projected run segment to Langfuse. Failures remain
/// best-effort and cannot change a workflow result.
pub(crate) async fn export_run_to_langfuse(
    config: &Config,
    flow_name: &str,
    flow_id: &str,
    run_id: &str,
    status: &str,
    trigger: FlowRunTrigger,
    observations: &[GraphObservation],
) {
    langfuse_export::export_flow_run_trace(
        config,
        flow_name,
        flow_id,
        run_id,
        status,
        trigger,
        observations,
    )
    .await;
}

/// Surface a parked run as a desktop approval notification.
pub(crate) fn notify_pending_approval(flow: &Flow, run_id: &str, pending_approvals: &[String]) {
    if pending_approvals.is_empty() {
        return;
    }

    use crate::desktop::notifications::bus::publish_core_notification;
    use crate::desktop::notifications::types::{
        CoreNotificationAction, CoreNotificationCategory, CoreNotificationEvent,
    };

    let action_payload = json!({
        "flow_id": flow.id,
        "thread_id": run_id,
        "node_ids": pending_approvals,
    });

    publish_core_notification(CoreNotificationEvent {
        id: format!("flow-pending-approval:{}:{}", flow.id, run_id),
        category: CoreNotificationCategory::Agents,
        title: "Workflow needs approval".to_string(),
        body: format!(
            "\"{}\" is waiting on {} approval{} before it can continue.",
            flow.name,
            pending_approvals.len(),
            if pending_approvals.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
        deep_link: None,
        timestamp_ms: now_ms(),
        actions: Some(vec![CoreNotificationAction {
            action_id: "approve".to_string(),
            label: "Review".to_string(),
            payload: Some(action_payload),
        }]),
        workspace: None,
        workspace_revision: None,
    });
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
