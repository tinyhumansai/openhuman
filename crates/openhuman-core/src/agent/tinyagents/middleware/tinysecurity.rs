//! `TinySecurityPolicy`: the single seam that maps the tinysecurity module's
//! [`Verdict`] onto the harness [`PolicyDecision`] (issue #7328, phase 6).
//!
//! The module decides; this adapter only translates. It is a
//! [`ToolCallPolicy`] so the harness `ToolPolicyGate` can sit before every
//! effect-capable middleware. A module fault never allows: the decider
//! returns `Err` and the call is denied with no retry.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::middleware::{PolicyDecision, ToolCallPolicy};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinysecurity_bus::{DenialReason, Verdict};

use crate::agent::tinyagents::host::OpenHumanRunContext;

/// Boxed async decider: module verdict for one tool call, or a fault string.
pub(crate) type VerdictFuture = Pin<Box<dyn Future<Output = Result<Verdict, String>> + Send>>;
pub(crate) type VerdictDecider = Arc<dyn Fn(&TaToolCall) -> VerdictFuture + Send + Sync>;

/// Stable, payload-free text for a denial category.
fn reason_text(reason: &DenialReason) -> &'static str {
    match reason {
        DenialReason::Floor => "blocked by the security floor",
        DenialReason::UnsupportedEffect => "unsupported effect or unknown tool",
        DenialReason::ApprovalUnavailable => "approval is unavailable",
        DenialReason::Access => "insufficient access",
        DenialReason::PathScope => "path is outside the allowed scope",
        DenialReason::PathResolution => "path could not be securely resolved",
        DenialReason::Command => "command is not safely supported",
        DenialReason::InvalidContext => "missing verified caller identity",
        DenialReason::Content => "content blocked by the security scanner",
    }
}

/// Map a module verdict to the harness decision. `RequireApproval` carries the
/// persisted request id in its reason so the approval resolver can bind to it.
pub(crate) fn verdict_to_policy_decision(verdict: &Verdict) -> PolicyDecision {
    match verdict {
        Verdict::Allow => PolicyDecision::Allow,
        Verdict::Deny { reason } => PolicyDecision::Deny {
            reason: reason_text(reason).to_string(),
        },
        Verdict::RequireApproval { request_id } => PolicyDecision::RequireApproval {
            reason: format!("approval required (request {request_id})"),
        },
    }
}

/// Fold several decisions; deny dominates, then require-approval, then allow.
pub(crate) fn combine_decisions(decisions: Vec<PolicyDecision>) -> PolicyDecision {
    let mut out = PolicyDecision::Allow;
    for d in decisions {
        match (&out, &d) {
            (PolicyDecision::Deny { .. }, _) => {}
            (_, PolicyDecision::Deny { .. }) => out = d,
            (PolicyDecision::RequireApproval { .. }, _) => {}
            (_, PolicyDecision::RequireApproval { .. }) => out = d,
            _ => {}
        }
    }
    out
}

pub(crate) struct TinySecurityPolicy {
    decider: VerdictDecider,
}

impl TinySecurityPolicy {
    pub(crate) fn new(decider: VerdictDecider) -> Self {
        Self { decider }
    }
}

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for TinySecurityPolicy {
    fn name(&self) -> &str {
        "tinysecurity"
    }

    async fn check(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> PolicyDecision {
        match (self.decider)(call).await {
            Ok(verdict) => {
                let decision = verdict_to_policy_decision(&verdict);
                tracing::debug!(tool = %call.name, "[tinyagents::mw] tinysecurity verdict mapped");
                decision
            }
            Err(_fault) => {
                tracing::warn!(tool = %call.name, "[tinyagents::mw] tinysecurity module fault; denying");
                PolicyDecision::Deny {
                    reason: "security module unavailable".to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tinysecurity_tests.rs"]
mod tests;
