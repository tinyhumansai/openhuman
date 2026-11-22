//! Whether a sub-agent of a scheduled-actions cron run may act.

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent_registry::host_definition::AgentDefinition;
use crate::cron::origin::ScheduledActionsRefusal;
use tinyagents_definition::SandboxMode;

pub(super) struct ChildGrantFacts<'a> {
    pub(super) under_scheduled_actions_root: bool,
    pub(super) child_depth: usize,
    pub(super) definition: &'a AgentDefinition,
    pub(super) root_workspace: Option<&'a tinytools::WorkspaceDescriptor>,
    pub(super) child_workspace: Option<&'a tinytools::WorkspaceDescriptor>,
    pub(super) saas: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildGrantRefusal {
    NotUnderScheduledActionsRoot,
    NotADirectChild,
    Definition(ScheduledActionsRefusal),
    NotSandboxed,
    OtherWorkspace,
}

pub(super) fn scheduled_actions_child_grant(
    _facts: &ChildGrantFacts<'_>,
) -> Result<(), ChildGrantRefusal> {
    Err(ChildGrantRefusal::NotUnderScheduledActionsRoot)
}

pub(super) fn grant_scheduled_actions_to_child(
    _child: &mut OpenHumanRunContext,
    _definition: &AgentDefinition,
    _spawn_workspace: Option<&tinytools::WorkspaceDescriptor>,
) -> bool {
    false
}

#[cfg(test)]
#[path = "scheduled_actions_grant_tests.rs"]
mod tests;
