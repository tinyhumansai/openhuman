//! Host context extraction for TinyAgents' per-call tool wrapper.
//!
//! Network disclosures, x402 hooks and artifact UI writes need the chat and
//! dispatch context belonging to the invocation that constructs their tool.
//! `tinyagents_harness_tools::per_call::PerCallTool` owns the generic wrapper;
//! this module only selects OpenHuman's values from `ToolRunContext`.

use tinytools::ToolRunContext;

use crate::agent::tinyagents::host::run_context::{approval_for_tool, dispatch_ctx_for_tool};
use crate::core::runtime::DispatchCtx;
use crate::security::approval::ApprovalChatContext;

pub(crate) use tinyagents_harness_tools::per_call::PerCallTool;

/// The OpenHuman values a per-call tool needs from one invocation.
pub(crate) struct PerCallContext {
    pub chat: Option<ApprovalChatContext>,
    pub dispatch: DispatchCtx,
}

/// Extract this call's chat routing and derived dispatch context.
pub(crate) fn context(context: Option<&dyn ToolRunContext>) -> PerCallContext {
    let approval = approval_for_tool(context);
    PerCallContext {
        chat: approval.chat,
        dispatch: dispatch_ctx_for_tool(context),
    }
}

#[cfg(test)]
#[path = "per_call_tests.rs"]
mod tests;
