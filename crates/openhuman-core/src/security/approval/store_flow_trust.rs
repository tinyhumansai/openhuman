//! Flow-trust persistence split out of `store.rs` (see that file's module
//! doc) purely to keep it under the repo's per-file line budget.
//!
//! Covers the save-time flow pre-authorization audit row plus the
//! `flow_tool_trust` table: per-`(flow_id, tool_name)` "approve always for
//! this flow" grants used by `ApprovalGate::intercept_audited` to
//! short-circuit parking for a trusted flow/tool pair.

use anyhow::Result;

use crate::config::Config;

use super::super::types::{ApprovalDecision, ApprovalSourceContext};

/// The action summary of a flow pre-authorization audit row.
const PREAUTHORIZATION_SUMMARY: &str = "Pre-authorized for this flow when it was saved and enabled";

/// Record a save-time flow pre-authorization in the durable audit trail as a
/// born-decided row (`decided_at = created_at`, decision
/// `approve_always_for_flow`): it never appears in `list_pending` (which
/// filters `decided_at IS NULL`) but does surface in
/// `list_recent_decisions`, so Settings → Approval history shows exactly
/// when and for which tool the user granted blanket trust. The
/// `source_context` carries an empty `run_id` — no run existed yet — which
/// also keeps it invisible to `list_pending_for_flow_run`.
pub fn record_flow_preauthorization(
    config: &Config,
    flow_id: &str,
    tool_name: &str,
    session_id: &str,
) -> Result<()> {
    super::super::store_documents::current(config)?.insert_decided(
        tool_name,
        PREAUTHORIZATION_SUMMARY,
        session_id,
        &ApprovalSourceContext::Flow {
            flow_id: flow_id.to_string(),
            run_id: String::new(),
            node_id: None,
        },
        ApprovalDecision::ApproveAlwaysForFlow,
    )
}

/// Grant "approve always for this flow" trust to a `(flow_id, tool_name)`
/// pair — inserted when the user picks `ApproveAlwaysForFlow` on a
/// flow-origin park. `INSERT OR IGNORE` makes re-granting an already-trusted
/// pair a harmless no-op rather than a primary-key error.
pub fn insert_flow_trust(config: &Config, flow_id: &str, tool_name: &str) -> Result<()> {
    super::super::store_documents::current(config)?.insert_flow_trust(flow_id, tool_name)
}

/// List every `tool_name` currently holding "approve always for this flow"
/// trust for `flow_id`, ordered by name for stable output. Used by the
/// save-time pre-authorization manifest (`flows_approval_manifest`) to diff
/// "what the graph needs" against "what is already granted".
pub fn list_flow_trust(config: &Config, flow_id: &str) -> Result<Vec<String>> {
    super::super::store_documents::current(config)?.list_flow_trust(flow_id)
}

/// Delete flow trust rows for `flow_id`. With `tool_names: None` every grant
/// for the flow is removed (flow deletion cleanup); with `Some(names)` only
/// the named grants are revoked. Returns the number of rows removed. Deleting
/// a name that was never granted is a no-op, keeping the call idempotent.
pub fn delete_flow_trust(
    config: &Config,
    flow_id: &str,
    tool_names: Option<&[String]>,
) -> Result<usize> {
    super::super::store_documents::current(config)?.delete_flow_trust(flow_id, tool_names)
}

/// Whether `(flow_id, tool_name)` was previously granted "approve always for
/// this flow" trust. Consulted by [`super::gate::ApprovalGate::intercept_audited`]
/// before parking a `Workflow`-origin tool call.
pub fn is_flow_tool_trusted(config: &Config, flow_id: &str, tool_name: &str) -> Result<bool> {
    super::super::store_documents::current(config)?.is_flow_tool_trusted(flow_id, tool_name)
}
