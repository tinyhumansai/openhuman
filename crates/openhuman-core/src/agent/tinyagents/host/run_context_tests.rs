use super::*;

#[test]
fn converts_explicit_values_to_the_canonical_tinyagents_context() {
    let cancellation = tinyagents_harness::cancel::CancellationToken::new();
    let workspace = tinytools::WorkspaceDescriptor::new("/work/action");
    let mut host = OpenHumanRunContext::new()
        .with_cancellation(cancellation.clone())
        .with_workspace(workspace.clone());
    host.thread_id = Some("thread-a".to_string());

    let run = host.into_tinyagents(tinyagents_harness::context::RunConfig::new("run-a"));

    assert_eq!(run.thread_id().map(|id| id.as_str()), Some("thread-a"));
    assert_eq!(run.workspace, Some(workspace));
    cancellation.cancel();
    assert!(run.cancellation.is_cancelled());
}

#[test]
fn child_inherits_tree_handles_but_isolates_observations_and_usage() {
    let mut parent = OpenHumanRunContext::new();
    parent.thread_id = Some("thread-a".to_string());
    parent.file_state_agent_id = Some("parent-file-state".to_string());
    *parent.resolved_route.lock().expect("route lock") =
        Some(tinyinference_llm::model::ResolvedModelRoute {
            provider: "provider-a".to_string(),
            model: "model-a".to_string(),
            route: "chat-v1".to_string(),
        });

    let child = parent.child();

    assert_eq!(child.spawn_depth, 1);
    assert_eq!(child.thread_id.as_deref(), Some("thread-a"));
    assert!(child.file_state_agent_id().is_none());
    assert!(
        child
            .resolved_route
            .lock()
            .expect("child route lock")
            .is_none()
    );
    assert!(
        child
            .subagent_usage
            .lock()
            .expect("child usage lock")
            .is_empty()
    );

    parent.cancellation.cancel();
    assert!(child.cancellation.is_cancelled());
}

#[test]
fn root_context_is_owned_and_children_inherit_thread() {
    let mut root = OpenHumanRunContext::new();
    root.thread_id = Some("thread-a".to_owned());
    assert_eq!(root.child().thread_id.as_deref(), Some("thread-a"));
}

#[test]
fn dispatch_guard_refuses_pause_and_budget_without_a_task_scope() {
    let paused = TurnDispatchState::new(Some(std::time::Duration::from_secs(60)));
    paused.record_pause_requested(15, 15);
    assert_eq!(
        paused.check(),
        DispatchDecision::RefusePaused {
            completed_model_calls: 15,
            cap: 15,
        }
    );

    // A zero budget makes the time relationship deterministic: after an
    // observed child, any check is strictly short of that child's duration.
    let exhausted = TurnDispatchState::new(Some(std::time::Duration::ZERO));
    exhausted.record_subagent_elapsed(std::time::Duration::from_secs(1));
    assert!(matches!(
        exhausted.check(),
        DispatchDecision::RefuseBudget {
            observed_max_ms: 1_000,
            observed_samples: 1,
            ..
        }
    ));
}

#[test]
fn child_ledgers_are_isolated_and_parent_keeps_completed_child_totals() {
    let root = OpenHumanRunContext::new();
    let left = root.child();
    let right = root.child();
    left.append_subagent_usage(SubagentUsageEntry {
        task_id: "left-task".into(),
        agent_id: "researcher".into(),
        usage: crate::agent::harness::subagent_runner::SubagentUsage {
            input_tokens: 3,
            output_tokens: 2,
            cached_input_tokens: 1,
            charged_amount_usd: 0.01,
        },
    });
    assert!(right.subagent_usage_entries().is_empty());

    left.record_completed_subagent_usage(SubagentUsageEntry {
        task_id: "completed-child".into(),
        agent_id: "researcher".into(),
        usage: crate::agent::harness::subagent_runner::SubagentUsage::default(),
    });
    assert_eq!(root.subagent_usage_entries().len(), 1);
    assert_eq!(left.subagent_usage_entries().len(), 1);
}

#[test]
fn failed_outer_run_promotes_completed_nested_usage_once() {
    let root = OpenHumanRunContext::new();
    let outer = root.child();
    outer.append_subagent_usage(SubagentUsageEntry {
        task_id: "nested".into(),
        agent_id: "researcher".into(),
        usage: crate::agent::harness::subagent_runner::SubagentUsage {
            input_tokens: 7,
            ..Default::default()
        },
    });

    // This is the failure/cancellation finalizer path. Calling it once leaves
    // the root with the completed nested work even though the outer run has no
    // terminal outcome of its own.
    outer.promote_completed_descendant_usage();
    assert_eq!(root.subagent_usage_entries().len(), 1);
    assert_eq!(root.subagent_usage_entries()[0].task_id, "nested");
}

#[test]
fn detached_children_reset_turn_accounting_and_cancellation() {
    let mut root = OpenHumanRunContext::new();
    root.thread_id = Some("thread-a".into());
    let dispatch = Arc::new(TurnDispatchState::new(Some(std::time::Duration::ZERO)));
    dispatch.record_subagent_elapsed(std::time::Duration::from_secs(1));
    root.dispatch = Some(dispatch);
    let detached = root.detached_child();

    assert_eq!(detached.thread_id.as_deref(), Some("thread-a"));
    assert!(detached.dispatch.is_none());
    root.cancellation.cancel();
    assert!(!detached.cancellation.is_cancelled());
    detached.record_completed_subagent_usage(SubagentUsageEntry {
        task_id: "background".into(),
        agent_id: "archivist".into(),
        usage: Default::default(),
    });
    assert!(root.subagent_usage_entries().is_empty());
    assert_eq!(detached.subagent_usage_entries().len(), 1);
}

#[test]
fn children_inherit_attachment_placeholders() {
    let mut root = OpenHumanRunContext::new();
    root.attachment_placeholders = std::sync::Arc::new(vec!["[Image: x #att:1]".into()]);
    assert_eq!(
        root.child().attachment_placeholders.as_slice(),
        ["[Image: x #att:1]"]
    );
}
