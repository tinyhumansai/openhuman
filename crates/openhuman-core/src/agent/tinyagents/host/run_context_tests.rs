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
fn concurrent_root_configs_are_unique_while_one_turn_keeps_its_id() {
    let left = std::thread::spawn(|| fresh_root_run_config("openhuman-session"));
    let right = std::thread::spawn(|| fresh_root_run_config("openhuman-session"));
    let left = left.join().expect("left root config");
    let right = right.join().expect("right root config");
    assert_ne!(left.run_id, right.run_id);

    let mut host = OpenHumanRunContext::new();
    let first = host.root_run_config("openhuman-session");
    let same_turn_boundary = host.root_run_config("openhuman-agent-turn");
    assert_eq!(first.run_id, same_turn_boundary.run_id);
}

#[test]
fn direct_subagent_child_derives_explicit_key_before_owned_lineage_child() {
    let mut host = OpenHumanRunContext::new();
    host.thread_id = Some("thread-a".into());
    let parent = host.into_tinyagents(
        tinyagents_harness::context::RunConfig::new("root-run").with_thread("thread-a"),
    );

    let (key, child) = direct_subagent_child(
        &parent,
        "task-42",
        tinyagents_harness::context::RunConfig::new("child-execution-42"),
    )
    .expect("direct child");

    assert_eq!(key.root_run_id, "root-run");
    assert_eq!(key.parent_run_id, "root-run");
    assert_eq!(key.thread_id.as_deref(), Some("thread-a"));
    assert_eq!(key.task_id, "task-42");
    assert_eq!(child.run_id().as_str(), "child-execution-42");
    assert_eq!(child.lineage().root_run_id.as_str(), "root-run");
    assert_eq!(
        child.lineage().parent_run_id.as_ref(),
        Some(parent.run_id())
    );
    assert_eq!(child.thread_id().map(|id| id.as_str()), Some("thread-a"));
}

#[test]
fn direct_subagent_child_shares_tinyagents_and_host_cancellation_tree() {
    let parent = OpenHumanRunContext::new()
        .into_tinyagents(tinyagents_harness::context::RunConfig::new("root-cancel"));
    let (_, child) = direct_subagent_child(
        &parent,
        "cancel-task",
        tinyagents_harness::context::RunConfig::new("child-cancel"),
    )
    .expect("direct child");

    parent.cancellation.cancel();
    assert!(child.cancellation.is_cancelled());
    assert!(child.data.cancellation.is_cancelled());
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
    assert!(child
        .resolved_route
        .lock()
        .expect("child route lock")
        .is_none());
    assert!(child
        .subagent_usage
        .lock()
        .expect("child usage lock")
        .is_empty());

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
        usage: crate::agent::subagent_host::SubagentUsage {
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
        usage: crate::agent::subagent_host::SubagentUsage::default(),
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
        usage: crate::agent::subagent_host::SubagentUsage {
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

/// A parent snapshot as a cached web-chat session produces one: the prelude
/// captured `on_progress` before `set_on_progress` ran, so it carries `None`.
fn stale_parent_snapshot() -> ParentExecutionContext {
    ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: std::collections::HashSet::new(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(vec!["done"]),
        )),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: std::path::PathBuf::from("/tmp/openhuman-attach-parent"),
        workspace_descriptor: None,
        agent_config: crate::config::AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "parent-session".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: crate::agent::prompts::ToolCallFormat::Native,
        session_key: "parent-key".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

/// Round-trips one real `SubagentSpawned` through `sink` and asserts it lands
/// on `rx`, the receiver belonging to the run whose sink the bind was supposed
/// to install.
///
/// `on_progress.is_some()` proves *presence*, not *identity*: a bind that
/// installed some other, freshly created channel satisfies it just as happily,
/// and that is the miswiring that actually ships. Production reads this exact
/// field one clone away — `spawn_async_subagent_execute.rs` opens with
/// `let progress_sink = parent.on_progress.clone();` — so delivery onto this
/// receiver is what "the panel will see the spawn" reduces to.
fn assert_bound_sink_is_this_runs_channel(
    sink: Option<&Sender<AgentProgress>>,
    rx: &mut tokio::sync::mpsc::Receiver<AgentProgress>,
    case: &str,
) {
    let sink = sink.unwrap_or_else(|| panic!("{case}: no progress sink bound at all"));
    sink.try_send(AgentProgress::SubagentSpawned {
        agent_id: "probe-agent".to_string(),
        task_id: "probe-task".to_string(),
        mode: "async".to_string(),
        dedicated_thread: false,
        prompt_chars: 5,
        worker_thread_id: None,
        display_name: None,
        prompt: "probe".to_string(),
        parent_call_id: None,
    })
    .unwrap_or_else(|err| panic!("{case}: the bound sink refused the event: {err}"));

    match rx.try_recv() {
        Ok(AgentProgress::SubagentSpawned { task_id, .. }) => assert_eq!(
            task_id, "probe-task",
            "{case}: a different event arrived on this run's receiver"
        ),
        other => panic!(
            "{case}: the bound sink is not this run's channel — nothing arrived \
             on its receiver ({other:?}). A sub-agent spawned through this \
             parent would be dropped and the Background tasks panel would stay \
             empty, exactly as before the fix."
        ),
    }
}

/// Sub-agent spawn/completion are the only progress events that ride the
/// parent snapshot rather than the harness event projection. The snapshot's
/// own sink is `None` for the whole life of a checked-out session, so binding
/// the run's live sink here is what keeps `subagent_spawned` reaching the
/// progress bridge — and with it the run-ledger row and the "Background tasks"
/// panel. Without the bind the panel reads "none running" while sub-agents run.
#[test]
fn attach_parent_binds_the_runs_live_progress_sink_over_a_stale_snapshot() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut context = OpenHumanRunContext::new();
    context.progress = Some(tx);

    context.attach_parent(stale_parent_snapshot());

    let bound = context.parent.as_ref().expect("parent installed");
    assert_bound_sink_is_this_runs_channel(
        bound.on_progress.as_ref(),
        &mut rx,
        "the parent snapshot handed to tools must carry the turn's own live \
         progress sink",
    );
}

/// The returned reference is the bound context, not the caller's snapshot.
///
/// `enrich_request` launches the triggered memory sub-agent *before* the rest
/// of the turn is assembled, so it has to hand that spawn a parent context.
/// Taking it from this return value is what stops it passing the stale
/// snapshot it started from — the same `on_progress = None` that kept
/// `subagent_spawned` out of the Background tasks panel. If this method ever
/// stops returning the installed parent, that call site silently regresses.
#[test]
fn attach_parent_returns_the_bound_parent_not_the_caller_snapshot() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut context = OpenHumanRunContext::new();
    context.progress = Some(tx);
    let snapshot = stale_parent_snapshot();
    assert!(snapshot.on_progress.is_none(), "snapshot starts stale");

    let bound = context.attach_parent(snapshot);

    assert_bound_sink_is_this_runs_channel(
        bound.on_progress.as_ref(),
        &mut rx,
        "the context returned by attach_parent must already carry the run's \
         own live sink, because enrich_request spawns from this value before \
         the turn is assembled",
    );
}

/// The bind must not blank a sink the snapshot did carry: a turn with no
/// progress subscriber of its own (CLI, cron) leaves the snapshot intact.
#[test]
fn attach_parent_keeps_the_snapshot_sink_when_the_run_has_none() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut snapshot = stale_parent_snapshot();
    snapshot.on_progress = Some(tx);
    let mut context = OpenHumanRunContext::new();
    assert!(context.progress.is_none(), "run has no sink of its own");

    context.attach_parent(snapshot);

    let bound = context.parent.as_ref().expect("parent installed");
    assert_bound_sink_is_this_runs_channel(
        bound.on_progress.as_ref(),
        &mut rx,
        "a run without its own sink must keep the snapshot's, and keep the \
         CLI/cron receiver that snapshot was carrying",
    );
}
