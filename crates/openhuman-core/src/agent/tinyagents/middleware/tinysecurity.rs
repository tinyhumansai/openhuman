//! `TinySecurityPolicy`: the single seam that maps the tinysecurity module's
//! [`Verdict`] onto the harness [`PolicyDecision`] (issue #7328, phase 6).
//!
//! The module decides; this adapter only translates. It is a
//! [`ToolCallPolicy`] so the harness `ToolPolicyGate` can sit before every
//! effect-capable middleware. A module fault never allows: the decider
//! returns `Err` and the call is denied with no retry.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::middleware::{PolicyDecision, ToolCallPolicy};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinysecurity_bus::{DenialReason, Verdict};

use crate::agent::tinyagents::host::OpenHumanRunContext;

/// Where a verdict for one tool call comes from. Production asks the
/// TinySecurity module over its bus contract ([`super::tinysecurity_module`]);
/// the seam exists so the mapping and precedence below are testable without a
/// loaded module. `Err` is a module fault: it never allows and is never retried.
#[async_trait]
pub(crate) trait VerdictSource: Send + Sync {
    async fn verdict(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> Result<Verdict, String>;

    /// Model-facing text for a `Deny` this source issued, when the generic text of
    /// the category would misdescribe it. `None` uses the category text.
    fn denial_text(&self, _call: &TaToolCall, _verdict: &Verdict) -> Option<String> {
        None
    }
}

/// The tool whose arguments carry a shell command string. The module gates it
/// by command class, so its refusals read as the command refusals the shell
/// tool itself produces.
pub(crate) const SHELL_TOOL: &str = "shell";

/// Stable, payload-free text for a denial category. `tool` names the refused
/// call; the module's answer carries no more than the category and, for the
/// hourly budget, the configured limit.
///
/// Model-facing refusals that predate the module keep their wording: a shell
/// refusal reads as the command refusal the shell tool gives, the read-only
/// tier and the protected-path floor carry the `[policy-blocked]` marker, a
/// scheduled turn carries `[policy-denied]`, and the budget text names its limit.
fn reason_text(tool: &str, reason: &DenialReason) -> String {
    use crate::security::policy::native_commands::refusal_message;
    use crate::security::policy::Refusal;
    use tinytools::status::{POLICY_BLOCKED_MARKER, POLICY_DENIED_MARKER};

    let shell = tool == SHELL_TOOL;
    match reason {
        DenialReason::Floor if shell => refusal_message(Refusal::Protected),
        DenialReason::Floor => "blocked by the security floor".into(),
        DenialReason::UnsupportedEffect => "unsupported effect or unknown tool".into(),
        DenialReason::ApprovalUnavailable => "human approval is required".into(),
        DenialReason::Access if shell => refusal_message(Refusal::Category),
        DenialReason::Access => "insufficient access".into(),
        DenialReason::PathScope => "path is outside the allowed scope".into(),
        DenialReason::PathResolution => "path could not be securely resolved".into(),
        DenialReason::Command if shell => refusal_message(Refusal::Hidden),
        DenialReason::Command => "command is not safely supported".into(),
        DenialReason::InvalidContext => "missing verified caller identity".into(),
        DenialReason::Content => "content blocked by the security scanner".into(),
        DenialReason::ReadOnlyMode if shell => refusal_message(Refusal::ReadOnly),
        DenialReason::ReadOnlyMode => format!(
            "{POLICY_BLOCKED_MARKER} Security policy: read-only mode, cannot perform '{tool}'."
        ),
        DenialReason::RateLimited { max_per_hour } => format!(
            "Rate limit exceeded: action budget exhausted ({max_per_hour} actions/hour). Wait \
             for the rolling one-hour window to refill, or raise the limit in Settings -> \
             Advanced -> Agent autonomy."
        ),
        DenialReason::ScheduledReadOnly => format!(
            "{POLICY_DENIED_MARKER} Scheduled agent turns may use read-only tools only; \
             '{tool}' needs a live user action."
        ),
        DenialReason::HighRisk => refusal_message(Refusal::HighRisk),
        DenialReason::Privacy => format!(
            "{POLICY_BLOCKED_MARKER} The registered privacy mode does not allow '{tool}' to send \
             this data off the device."
        ),
    }
}

/// Map a module verdict for the call to `tool` to the harness decision. `RequireApproval` carries the
/// persisted request id in its reason so the approval resolver can bind to it.
///
/// `Deny { ApprovalUnavailable }` is the module's plain `Evaluate` answer for a
/// host tool whose only remaining requirement is a human ("the weakest answer":
/// every concrete check passed). It maps to `RequireApproval`, never to `Allow`;
/// the gate fails closed when no approval resolver is attached, and the
/// resolver itself denies when no human can be asked. Every other denial stays
/// a denial.
pub(crate) fn verdict_to_policy_decision(tool: &str, verdict: &Verdict) -> PolicyDecision {
    match verdict {
        Verdict::Allow => PolicyDecision::Allow,
        Verdict::Deny {
            reason: DenialReason::ApprovalUnavailable,
        } => PolicyDecision::RequireApproval {
            reason: reason_text(tool, &DenialReason::ApprovalUnavailable),
        },
        Verdict::Deny { reason } => PolicyDecision::Deny {
            reason: reason_text(tool, reason),
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

/// The one security policy every agent path registers. The module decides; this
/// translates. Several sources fold with [`combine_decisions`]: a fault from any
/// source is a denial, so a source can only narrow the answer.
pub(crate) struct TinySecurityPolicy {
    sources: Vec<Arc<dyn VerdictSource>>,
    policy_sources: Vec<Arc<dyn ToolCallPolicy<OpenHumanRunContext>>>,
}

impl TinySecurityPolicy {
    pub(crate) fn new(source: Arc<dyn VerdictSource>) -> Self {
        Self::with_sources(vec![source])
    }

    pub(crate) fn with_sources(sources: Vec<Arc<dyn VerdictSource>>) -> Self {
        Self {
            sources,
            policy_sources: Vec::new(),
        }
    }

    /// Add a host policy adapter to the same decision fold as module verdicts.
    pub(crate) fn with_policy_source(
        mut self,
        source: Arc<dyn ToolCallPolicy<OpenHumanRunContext>>,
    ) -> Self {
        self.policy_sources.push(source);
        self
    }
}

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for TinySecurityPolicy {
    fn name(&self) -> &str {
        "tinysecurity"
    }

    async fn check(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> PolicyDecision {
        let mut decisions = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            match source.verdict(ctx, call).await {
                Ok(verdict) => {
                    tracing::debug!(tool = %call.name, "[tinyagents::mw] tinysecurity verdict mapped");
                    let decision = match (source.denial_text(call, &verdict), &verdict) {
                        (Some(reason), Verdict::Deny { .. }) => PolicyDecision::Deny { reason },
                        _ => verdict_to_policy_decision(&call.name, &verdict),
                    };
                    decisions.push(decision);
                }
                // A fault never allows and is never retried: the call is denied
                // here, without asking the remaining sources.
                Err(_fault) => {
                    tracing::warn!(tool = %call.name, "[tinyagents::mw] tinysecurity module fault; denying");
                    return PolicyDecision::Deny {
                        reason: "security module unavailable".to_string(),
                    };
                }
            }
        }
        for source in &self.policy_sources {
            decisions.push(source.check(ctx, call).await);
        }
        combine_decisions(decisions)
    }
}

#[cfg(test)]
#[path = "tinysecurity_tests.rs"]
mod tests;
