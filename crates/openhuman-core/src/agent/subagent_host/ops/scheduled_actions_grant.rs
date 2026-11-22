//! Whether a sub-agent of a scheduled-actions cron run may act.
//!
//! A cron job whose agent opted into scheduled actions runs its root as
//! `TrustedAutomation { CronActions }` with a marked run (`scheduled_actions`),
//! and its security gate skips the read-only cron stage. `child()` clears the
//! mark, so every sub-agent starts read-only. The runner calls
//! [`grant_scheduled_actions_to_child`] once per spawn, after the child's run
//! context is built, and re-marks that one child only when
//! [`scheduled_actions_child_grant`] says so. The gate still refuses anything
//! deeper than depth 1, and every later gate stage (autonomy tier, approvals,
//! wallet policy, action budget) runs for a granted child as for the root.

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent_registry::host_definition::AgentDefinition;
use crate::cron::origin::{scheduled_actions_definition_refusal, ScheduledActionsRefusal};
use tinyagents_definition::SandboxMode;

/// What the grant decision reads about one spawn.
pub(super) struct ChildGrantFacts<'a> {
    /// The tree's root is a scheduled-actions cron run
    /// (`OpenHumanRunContext::scheduled_actions_root`).
    pub(super) under_scheduled_actions_root: bool,
    /// The child's run depth (the cron root is 0).
    pub(super) child_depth: usize,
    /// The child's own agent definition.
    pub(super) definition: &'a AgentDefinition,
    /// The workspace descriptor the root runs in (`None`: the action dir).
    pub(super) root_workspace: Option<&'a tinytools::WorkspaceDescriptor>,
    /// The workspace descriptor the child will run in.
    pub(super) child_workspace: Option<&'a tinytools::WorkspaceDescriptor>,
    /// The process runs in SaaS mode.
    pub(super) saas: bool,
}

/// Why a child is not granted scheduled actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildGrantRefusal {
    /// The tree's root is not a scheduled-actions cron run.
    NotUnderScheduledActionsRoot,
    /// The child is not a direct child (depth 1) of the root.
    NotADirectChild,
    /// The child's definition fails the root's own definition checks.
    Definition(ScheduledActionsRefusal),
    /// The child's definition is not `sandbox_mode = "sandboxed"`.
    NotSandboxed,
    /// The child would run in a workspace other than the root's.
    OtherWorkspace,
}

/// Whether a spawned child may lift the read-only cron stage like its root.
///
/// Granted only when ALL hold: the tree's root is a scheduled-actions run, the
/// child is at depth exactly 1, its own definition passes the root's
/// definition checks (`scheduled_actions = true`, a named tool scope, not
/// SaaS), it is sandboxed, and it runs in exactly the root's workspace.
pub(super) fn scheduled_actions_child_grant(
    facts: &ChildGrantFacts<'_>,
) -> Result<(), ChildGrantRefusal> {
    if !facts.under_scheduled_actions_root {
        return Err(ChildGrantRefusal::NotUnderScheduledActionsRoot);
    }
    if facts.child_depth != 1 {
        return Err(ChildGrantRefusal::NotADirectChild);
    }
    if let Some(refusal) = scheduled_actions_definition_refusal(facts.definition, facts.saas) {
        return Err(ChildGrantRefusal::Definition(refusal));
    }
    if facts.definition.sandbox_mode != SandboxMode::Sandboxed {
        return Err(ChildGrantRefusal::NotSandboxed);
    }
    if facts.child_workspace != facts.root_workspace {
        return Err(ChildGrantRefusal::OtherWorkspace);
    }
    Ok(())
}

/// Re-marks `child` for scheduled actions when [`scheduled_actions_child_grant`]
/// allows it, and returns whether it did.
///
/// `child` is the context `for_subagent` built from the parent's `child()`, so
/// its `workspace` is still the root's; `spawn_workspace` is the descriptor the
/// runner hands the child run, which replaces it when present. A granted child
/// keeps the root's `sandbox_filesystem_only` exactly as inherited: its own
/// definition's value is not applied, because files-only also opens the host
/// network to the shell and a child must not get more network than its root.
/// An ungranted child is left exactly as built.
pub(super) fn grant_scheduled_actions_to_child(
    child: &mut OpenHumanRunContext,
    definition: &AgentDefinition,
    spawn_workspace: Option<&tinytools::WorkspaceDescriptor>,
) -> bool {
    if !child.scheduled_actions_root {
        return false;
    }
    let root_workspace = child.workspace.as_ref();
    let decision = scheduled_actions_child_grant(&ChildGrantFacts {
        under_scheduled_actions_root: child.scheduled_actions_root,
        child_depth: child.spawn_depth,
        definition,
        root_workspace,
        child_workspace: spawn_workspace.or(root_workspace),
        saas: crate::core::runtime::is_saas(),
    });
    match decision {
        Ok(()) => {
            child.scheduled_actions = true;
            tracing::info!(
                agent_id = %definition.id,
                spawn_depth = child.spawn_depth,
                files_only = child.sandbox_filesystem_only,
                "[subagent_host] scheduled-actions grant: direct child may act"
            );
            true
        }
        Err(refusal) => {
            tracing::debug!(
                agent_id = %definition.id,
                spawn_depth = child.spawn_depth,
                ?refusal,
                "[subagent_host] scheduled-actions grant refused; child stays read-only"
            );
            false
        }
    }
}

#[cfg(test)]
#[path = "scheduled_actions_grant_tests.rs"]
mod tests;
