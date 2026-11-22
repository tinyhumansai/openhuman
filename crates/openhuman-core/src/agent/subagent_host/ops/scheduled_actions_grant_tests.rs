use super::*;
use crate::cron::origin::ScheduledActionsRefusal;

fn acting_child() -> AgentDefinition {
    let mut definition = AgentDefinition {
        id: "builder_worker".into(),
        tools: tinyagents_definition::ToolScope::Named(vec!["shell".into(), "file_write".into()]),
        sandbox_mode: SandboxMode::Sandboxed,
        ..Default::default()
    };
    definition.extensions.scheduled_actions = true;
    definition
}

fn root_workspace() -> tinytools::WorkspaceDescriptor {
    tinytools::WorkspaceDescriptor::new("/srv/workbench")
}

fn facts<'a>(
    definition: &'a AgentDefinition,
    root_workspace: Option<&'a tinytools::WorkspaceDescriptor>,
    child_workspace: Option<&'a tinytools::WorkspaceDescriptor>,
) -> ChildGrantFacts<'a> {
    ChildGrantFacts {
        under_scheduled_actions_root: true,
        child_depth: 1,
        definition,
        root_workspace,
        child_workspace,
        saas: false,
    }
}

#[test]
fn an_opted_in_sandboxed_direct_child_in_the_roots_workspace_is_granted() {
    let definition = acting_child();
    let workspace = root_workspace();
    assert_eq!(
        scheduled_actions_child_grant(&facts(&definition, Some(&workspace), Some(&workspace))),
        Ok(())
    );
    // A root on the process-wide action dir (no descriptor) and a child that
    // stays there too share the workspace.
    assert_eq!(
        scheduled_actions_child_grant(&facts(&definition, None, None)),
        Ok(())
    );
}

#[test]
fn a_child_outside_a_scheduled_actions_tree_is_not_granted() {
    let definition = acting_child();
    let mut facts = facts(&definition, None, None);
    facts.under_scheduled_actions_root = false;
    assert_eq!(
        scheduled_actions_child_grant(&facts),
        Err(ChildGrantRefusal::NotUnderScheduledActionsRoot)
    );
}

#[test]
fn only_a_direct_child_of_the_root_is_granted() {
    let definition = acting_child();
    for depth in [0, 2, 3] {
        let mut facts = facts(&definition, None, None);
        facts.child_depth = depth;
        assert_eq!(
            scheduled_actions_child_grant(&facts),
            Err(ChildGrantRefusal::NotADirectChild),
            "depth {depth}"
        );
    }
}

#[test]
fn a_child_whose_definition_does_not_opt_in_is_not_granted() {
    let mut definition = acting_child();
    definition.extensions.scheduled_actions = false;
    assert_eq!(
        scheduled_actions_child_grant(&facts(&definition, None, None)),
        Err(ChildGrantRefusal::Definition(
            ScheduledActionsRefusal::NotOptedIn
        ))
    );
}

#[test]
fn a_child_with_a_wildcard_tool_scope_is_not_granted() {
    let mut definition = acting_child();
    definition.tools = tinyagents_definition::ToolScope::Wildcard;
    assert_eq!(
        scheduled_actions_child_grant(&facts(&definition, None, None)),
        Err(ChildGrantRefusal::Definition(
            ScheduledActionsRefusal::WildcardTools
        ))
    );
}

#[test]
fn a_child_that_is_not_sandboxed_is_not_granted() {
    for mode in [SandboxMode::None, SandboxMode::ReadOnly] {
        let mut definition = acting_child();
        definition.sandbox_mode = mode;
        assert_eq!(
            scheduled_actions_child_grant(&facts(&definition, None, None)),
            Err(ChildGrantRefusal::NotSandboxed),
            "{mode:?}"
        );
    }
}

#[test]
fn a_child_given_another_workspace_is_not_granted() {
    let definition = acting_child();
    let root = root_workspace();
    let worktree = tinytools::WorkspaceDescriptor::new("/srv/worktrees/task-1");
    assert_eq!(
        scheduled_actions_child_grant(&facts(&definition, Some(&root), Some(&worktree))),
        Err(ChildGrantRefusal::OtherWorkspace)
    );
    // Leaving the action dir for a descriptor of its own is another workspace too.
    assert_eq!(
        scheduled_actions_child_grant(&facts(&definition, None, Some(&worktree))),
        Err(ChildGrantRefusal::OtherWorkspace)
    );
    // Same root, wider trusted roots: not the root's workspace.
    let widened = root_workspace().with_trusted_root("/srv/elsewhere");
    assert_eq!(
        scheduled_actions_child_grant(&facts(&definition, Some(&root), Some(&widened))),
        Err(ChildGrantRefusal::OtherWorkspace)
    );
}

#[test]
fn a_child_in_saas_mode_is_not_granted() {
    let definition = acting_child();
    let mut facts = facts(&definition, None, None);
    facts.saas = true;
    assert_eq!(
        scheduled_actions_child_grant(&facts),
        Err(ChildGrantRefusal::Definition(ScheduledActionsRefusal::Saas))
    );
}

fn direct_child_of_scheduled_actions_root(
    root_files_only: bool,
) -> crate::agent::tinyagents::host::OpenHumanRunContext {
    let mut root = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    root.scheduled_actions = true;
    root.scheduled_actions_root = true;
    root.sandbox_filesystem_only = root_files_only;
    let mut child = root.child();
    child.spawn_depth = 1;
    child
}

#[test]
fn a_granted_child_acts_and_keeps_a_files_only_jail_the_root_or_its_definition_asks_for() {
    for (root_files_only, definition_files_only, expected) in [
        (false, false, false),
        (true, false, true),
        (false, true, true),
        (true, true, true),
    ] {
        let mut definition = acting_child();
        definition.extensions.sandbox_filesystem_only = definition_files_only;
        let mut child = direct_child_of_scheduled_actions_root(root_files_only)
            .for_subagent(&definition, None);
        assert!(grant_scheduled_actions_to_child(&mut child, &definition, None));
        assert!(child.scheduled_actions);
        assert_eq!(
            child.sandbox_filesystem_only, expected,
            "root {root_files_only}, definition {definition_files_only}"
        );
    }
}

#[test]
fn an_ungranted_child_keeps_its_inherited_context_unchanged() {
    for root_files_only in [false, true] {
        let mut definition = acting_child();
        definition.extensions.scheduled_actions = false;
        definition.extensions.sandbox_filesystem_only = true;
        let mut child = direct_child_of_scheduled_actions_root(root_files_only)
            .for_subagent(&definition, None);
        assert!(!grant_scheduled_actions_to_child(&mut child, &definition, None));
        assert!(!child.scheduled_actions);
        assert_eq!(child.sandbox_filesystem_only, root_files_only);
    }
}

#[test]
fn a_child_spawned_into_another_workspace_is_not_granted_by_the_runner() {
    let definition = acting_child();
    let mut child =
        direct_child_of_scheduled_actions_root(false).with_workspace(root_workspace());
    let worktree = tinytools::WorkspaceDescriptor::new("/srv/worktrees/task-1");
    assert!(!grant_scheduled_actions_to_child(
        &mut child,
        &definition,
        Some(&worktree)
    ));
    assert!(!child.scheduled_actions);
    // The same descriptor the root runs in is the root's workspace.
    assert!(grant_scheduled_actions_to_child(
        &mut child,
        &definition,
        Some(&root_workspace())
    ));
}
