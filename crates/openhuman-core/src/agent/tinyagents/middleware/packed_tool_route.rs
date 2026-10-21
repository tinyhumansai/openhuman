//! Host router for the harness `PackedToolRouteMiddleware`: route a call that
//! names a withheld packed tool by its bare name through `use_skill`, instead
//! of answering "unknown tool" (#6276).
//!
//! The rewrite mechanism lives in the harness. This file keeps the policy
//! only: a tool is routed where the turn's tool-policy session lets it run
//! (`session.decision_for(name).blocks_execution()` is false), a pack owns it,
//! and that pack's group is `Withheld`. No session (sub-agent, channel/CLI
//! turns) means no routing, so the registration allowlist stays the boundary.

use tinyagents_harness_tools::skills::USE_SKILL;
use tinyagents_hooks::lifecycle::{PackedToolRouteMiddleware, PackedToolRouter};
use tinytools::ToolPolicySession;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::tools::toolpacks::{self, GroupMode};

/// This turn's tool-policy snapshot; `None` disables routing.
pub(crate) struct SessionPackRouter {
    session: Option<ToolPolicySession>,
}

impl PackedToolRouter<OpenHumanRunContext> for SessionPackRouter {
    fn pack_for(&self, _data: &OpenHumanRunContext, tool: &str) -> Option<String> {
        let session = self.session.as_ref()?;
        if session.decision_for(tool).blocks_execution() {
            return None;
        }
        let pack = toolpacks::pack_for_tool(tool)?;
        if toolpacks::groups::current().mode_for_tool(tool) != GroupMode::Withheld {
            return None;
        }
        Some(pack.id.to_string())
    }
}

/// Build the harness middleware over the names this turn registered and the
/// turn's tool-policy session, if it has one.
pub(crate) fn packed_tool_route_middleware(
    registered: impl IntoIterator<Item = String>,
    session: Option<ToolPolicySession>,
) -> PackedToolRouteMiddleware<OpenHumanRunContext, SessionPackRouter> {
    PackedToolRouteMiddleware::new(registered, USE_SKILL, SessionPackRouter { session })
}

#[cfg(test)]
#[path = "packed_tool_route_tests.rs"]
mod tests;
