//! Test support: ask the production host authorization stage about one tool.
//!
//! A tool does not police its own autonomy tier any more; the host stage
//! (`OpenHumanSecurityGate::authorize_tool`) classifies the call from the
//! tool's declared permission level and external effect and applies the
//! TinySecurity tier. A tool's read-only test therefore has to go through it.

use std::sync::Arc;

use tinyagents_harness::host::security_gate::{GateDecision, SecurityGate, ToolCallRequest};
use tinytools::Tool;

use super::OpenHumanSecurityGate;
use crate::security::policy::SecurityPolicy;

/// The gate's decision for `args` on `tool` under `policy`, with `tool` as the
/// only registered tool.
pub(crate) async fn authorize_one(
    policy: Arc<SecurityPolicy>,
    tool: Box<dyn Tool>,
    args: serde_json::Value,
) -> GateDecision {
    let name = tool.name().to_owned();
    OpenHumanSecurityGate::new(policy, vec![Arc::new(vec![tool])])
        .authorize_tool(&ToolCallRequest::new(name, args, "lead"))
        .await
        .expect("the host authorization stage answers")
}

/// Whether `decision` is a refusal, for the read-only tier assertions.
pub(crate) fn is_denied(decision: &GateDecision) -> bool {
    matches!(decision, GateDecision::Deny { .. })
}
