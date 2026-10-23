//! The channel tool policy's "ask a human first" verdict, as a [`VerdictSource`]
//! (issue #7328, phase 6).
//!
//! A channel can permit a tool only with human approval
//! ([`ToolPolicyAction::RequireApproval`]). The module cannot know that: it is
//! host configuration of the channel, not a property of the tool or the caller.
//! This source feeds it into the same gate as a verdict that "a human must
//! approve", so the one approval resolver parks it. A channel `Deny` is not
//! restated here: the security gate's session ceiling enforces it.
//!
//! `agent_tool_policy` files `RequireApproval` under the blocked tools beside
//! `Deny`, so the only thing that can lift it is a human actually approving.
//! With no approval gate installed (CLI, headless embed, tests) nobody can
//! approve, and falling back to allow there would turn the channel's strictest
//! non-deny setting into its weakest on exactly the hosts with no human present,
//! so the call is denied instead.

use crate::security::approval::ApprovalGate;
use async_trait::async_trait;
use tinyagents_harness::context::RunContext;
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinysecurity_bus::{DenialReason, Verdict};
use tinytools::{ToolPolicyAction, ToolPolicySession};

use super::tinysecurity::VerdictSource;
use crate::agent::tinyagents::host::OpenHumanRunContext;

const LOG: &str = "[tinyagents::mw::tinysecurity_channel]";

/// What the model is told when the channel demands approval and none can be given.
const NO_APPROVAL_FLOW: &str = "This tool requires human approval, and no approval flow is \
                                available in this session.";

/// Feeds a session's channel `RequireApproval` decisions into the security gate.
pub(crate) struct ChannelApprovalVerdicts {
    session: ToolPolicySession,
}

impl ChannelApprovalVerdicts {
    pub(crate) fn new(session: ToolPolicySession) -> Self {
        Self { session }
    }

    fn demands_approval(&self, tool: &str) -> bool {
        matches!(
            self.session.decision_for(tool).action,
            ToolPolicyAction::RequireApproval
        )
    }
}

#[async_trait]
impl VerdictSource for ChannelApprovalVerdicts {
    async fn verdict(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> Result<Verdict, String> {
        if !self.demands_approval(&call.name) {
            return Ok(Verdict::Allow);
        }
        if ApprovalGate::try_global().is_none() {
            tracing::warn!(
                tool = %call.name,
                "{LOG} channel policy requires approval but no approval gate is installed; denying"
            );
            return Ok(Verdict::Deny {
                reason: DenialReason::Access,
            });
        }
        Ok(Verdict::Deny {
            reason: DenialReason::ApprovalUnavailable,
        })
    }

    fn denial_text(&self, call: &TaToolCall, verdict: &Verdict) -> Option<String> {
        (self.demands_approval(&call.name)
            && matches!(
                verdict,
                Verdict::Deny {
                    reason: DenialReason::Access
                }
            ))
        .then(|| NO_APPROVAL_FLOW.to_string())
    }
}

#[cfg(test)]
#[path = "tinysecurity_channel_tests.rs"]
mod tests;
