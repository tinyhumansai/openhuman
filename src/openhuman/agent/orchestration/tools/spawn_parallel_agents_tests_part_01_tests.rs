use super::*;

#[test]
fn metadata_methods_expose_execute_permission_and_schema() {
    let tool = SpawnParallelAgentsTool::default();
    assert_eq!(tool.name(), "spawn_parallel_agents");
    assert!(tool.description().contains("independent sub-agent tasks"));
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    let schema = tool.parameters_schema();
    assert_eq!(schema["required"][0], "tasks");
    assert_eq!(schema["properties"]["tasks"]["minItems"], 2);
}

#[test]
fn ownership_boundary_is_prepended_when_present() {
    let prompt = with_ownership_boundary("implement tests", Some("files: src/foo.rs"));
    assert!(prompt.starts_with("[Ownership Boundary]"));
    assert!(prompt.contains("files: src/foo.rs"));
    assert!(prompt.contains("[Task]\nimplement tests"));
}

#[test]
fn schema_advertises_isolation_and_base_ref() {
    let tool = SpawnParallelAgentsTool::default();
    let schema = tool.parameters_schema();
    let props = &schema["properties"]["tasks"]["items"]["properties"];
    assert_eq!(props["isolation"]["enum"][0], "none");
    assert_eq!(props["isolation"]["enum"][1], "worktree");
    assert_eq!(props["base_ref"]["enum"][0], "head");
    assert_eq!(props["base_ref"]["enum"][1], "fresh");
}

#[test]
fn task_deserializes_isolation_and_base_ref() {
    let task: ParallelAgentTask = serde_json::from_value(json!({
        "agent_id": "coder",
        "prompt": "do it",
        "isolation": "worktree",
        "base_ref": "fresh"
    }))
    .expect("deserialize task");
    assert_eq!(task.isolation.as_deref(), Some("worktree"));
    assert_eq!(task.base_ref.as_deref(), Some("fresh"));
}

#[test]
fn task_isolation_defaults_to_none() {
    let task: ParallelAgentTask = serde_json::from_value(json!({
        "agent_id": "researcher",
        "prompt": "read it"
    }))
    .expect("deserialize task");
    assert!(task.isolation.is_none());
    assert!(task.base_ref.is_none());
}

#[test]
fn result_omits_worktree_fields_when_absent() {
    let result = ParallelAgentResult {
        task_id: "t1".into(),
        agent_id: "a".into(),
        lineage: test_lineage("t1"),
        success: true,
        output: Some("ok".into()),
        error: None,
        ownership: None,
        elapsed_ms: 5,
        iterations: 1,
        stale_parent_reads: Vec::new(),
        worktree_path: None,
        changed_files: Vec::new(),
        dirty_status: None,
    };
    let v = serde_json::to_value(&result).unwrap();
    assert!(v.get("worktreePath").is_none());
    assert!(v.get("changedFiles").is_none());
    assert!(v.get("dirtyStatus").is_none());
}

#[test]
fn result_serializes_worktree_fields_when_present() {
    let result = ParallelAgentResult {
        task_id: "t2".into(),
        agent_id: "coder".into(),
        lineage: test_lineage("t2"),
        success: true,
        output: None,
        error: None,
        ownership: None,
        elapsed_ms: 9,
        iterations: 2,
        stale_parent_reads: Vec::new(),
        worktree_path: Some("/repo/.claude/worktrees/t2".into()),
        changed_files: vec!["src/a.rs".into()],
        dirty_status: Some(true),
    };
    let v = serde_json::to_value(&result).unwrap();
    assert_eq!(v["worktreePath"], "/repo/.claude/worktrees/t2");
    assert_eq!(v["changedFiles"][0], "src/a.rs");
    assert_eq!(v["dirtyStatus"], true);
}

#[tokio::test]
async fn rejects_single_task() {
    let tool = SpawnParallelAgentsTool::new();
    let result = tool
        .execute(json!({
            "tasks": [{ "agent_id": "researcher", "prompt": "only one" }]
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("at least two"));
}

#[tokio::test]
async fn rejects_missing_or_invalid_tasks_before_parent_lookup() {
    let tool = SpawnParallelAgentsTool::new();

    let missing = tool.execute(json!({})).await.expect_err("missing tasks");
    assert!(missing.to_string().contains("Missing 'tasks'"));

    let invalid = tool
        .execute(json!({ "tasks": "not an array" }))
        .await
        .expect_err("invalid tasks");
    assert!(invalid.to_string().contains("Invalid tasks array"));
}

#[tokio::test]
async fn rejects_two_tasks_outside_agent_turn() {
    let tool = SpawnParallelAgentsTool::new();
    let result = tool
        .execute(json!({
            "tasks": [
                { "agent_id": "researcher", "prompt": "one" },
                { "agent_id": "planner", "prompt": "two" }
            ]
        }))
        .await
        .expect("tool result");
    assert!(result.is_error);
    assert!(result.output().contains("outside of an agent turn"));
}

#[tokio::test]
async fn rejects_more_tasks_than_parent_parallel_limit() {
    // The parallel-limit check now runs inside the execution graph, which is
    // reached only after the registry lookup — so this test needs the global
    // builtins initialised (as its siblings already do) rather than relying on
    // whichever test happened to initialise them first.
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnParallelAgentsTool::new();
    let parent = parent_context(2);
    let result = with_parent_context(parent, async {
        tool.execute(json!({
            "tasks": [
                { "agent_id": "researcher", "prompt": "one" },
                { "agent_id": "planner", "prompt": "two" },
                { "agent_id": "critic", "prompt": "three" }
            ]
        }))
        .await
    })
    .await
    .expect("tool result");
    assert!(result.is_error);
    assert!(
        result.output().contains("max_parallel_tools"),
        "unexpected result: {}",
        result.output()
    );
}

#[tokio::test]
async fn collects_immediate_task_validation_failures() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnParallelAgentsTool::new();
    let parent = parent_context(4);

    let result = with_parent_context(parent, async {
        tool.execute(json!({
            "tasks": [
                { "agent_id": " ", "prompt": "missing agent", "ownership": "files: none" },
                { "agent_id": "__missing_agent__", "prompt": "unknown agent" },
                { "agent_id": "integrations_agent", "prompt": "needs toolkit" }
            ]
        }))
        .await
    })
    .await
    .expect("tool result");

    assert!(!result.is_error, "{}", result.output());
    let body: serde_json::Value = serde_json::from_str(&result.output()).expect("json output");
    assert_eq!(body["parallel_agents"]["total"], 3);
    assert_eq!(body["parallel_agents"]["failed"], 3);
    let errors = body["parallel_agents"]["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["error"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(errors
        .iter()
        .any(|error| error.contains("agent_id and prompt")));
    assert!(errors
        .iter()
        .any(|error| error.contains("unknown agent_id")));
    assert!(errors
        .iter()
        .any(|error| error.contains("requires toolkit")));
}

#[test]
fn shared_workspace_rejects_write_capable_named_worker_without_worktree() {
    let definition = definition_with_tool_scope(
        "researcher",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let definitions = HashMap::from([(definition.id.clone(), definition)]);
    let parent = parent_context_with_tools(
        4,
        vec![Box::new(PermissionFixtureTool {
            name: "write_fixture",
            level: PermissionLevel::Write,
        })],
    );

    let preflight = prepare_spawn_parallel_tasks_from_defs(
        vec![ParallelAgentTask {
            agent_id: "researcher".into(),
            prompt: "edit a file".into(),
            context: None,
            toolkit: None,
            ownership: None,
            isolation: None,
            base_ref: None,
        }],
        &definitions,
        &parent,
    );

    match preflight.into_iter().next().expect("one preflight result") {
        SpawnParallelTaskPreflight::Rejected(rejection) => {
            assert_eq!(rejection.kind, ParallelTaskRejectionKind::RequiresIsolation);
            assert!(rejection.error.contains("write_fixture:Write"));
            assert!(rejection.error.contains("isolation=\"worktree\""));
        }
        SpawnParallelTaskPreflight::Prepared(_) => {
            panic!("write-capable shared worker must require worktree isolation")
        }
    }
}

#[test]
fn shared_workspace_allows_readonly_or_explicitly_isolated_workers() {
    let readonly = definition_with_tool_scope(
        "researcher",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::ReadOnly,
    );
    let writer = definition_with_tool_scope(
        "critic",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let definitions = HashMap::from([(readonly.id.clone(), readonly), (writer.id.clone(), writer)]);
    let parent = parent_context_with_tools(
        4,
        vec![Box::new(PermissionFixtureTool {
            name: "write_fixture",
            level: PermissionLevel::Write,
        })],
    );

    let preflight = prepare_spawn_parallel_tasks_from_defs(
        vec![
            ParallelAgentTask {
                agent_id: "researcher".into(),
                prompt: "read only".into(),
                context: None,
                toolkit: None,
                ownership: None,
                isolation: None,
                base_ref: None,
            },
            ParallelAgentTask {
                agent_id: "critic".into(),
                prompt: "isolated edit".into(),
                context: None,
                toolkit: None,
                ownership: Some("files: src/b.rs".into()),
                isolation: Some("worktree".into()),
                base_ref: None,
            },
        ],
        &definitions,
        &parent,
    );

    assert!(preflight
        .into_iter()
        .all(|item| matches!(item, SpawnParallelTaskPreflight::Prepared(_))));
}

// This exercises the full parallel-subagent path (parent turn → spawn N
// subagents → each runs several nested tool-call iterations). It is a deep
// async state machine whose stacked frames exceed the default ~2 MiB libtest
// per-test thread stack in debug/coverage builds; the thread overflows and
// SIGABRTs the *entire* test process, which then non-deterministically tags an
// unrelated concurrently-running test as FAILED (issue #5209 — the
// experience-recall test was a frequent victim). CI only avoided this by
// exporting a 64 MiB `RUST_MIN_STACK`; a raw `cargo test` has no such env.
// Production drives agent turns on an explicit large stack for the same reason
// (`agent::bus::handle_agent_run_turn_on_large_stack`). Mirror that here so the
// test never aborts the process regardless of `RUST_MIN_STACK`.
#[test]
fn agent_turn_runs_long_parallel_subagent_flow_with_many_nested_tool_calls() {
    std::thread::Builder::new()
        .name("parallel-subagent-flow-test".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build large-stack test runtime")
                .block_on(
                    agent_turn_runs_long_parallel_subagent_flow_with_many_nested_tool_calls_inner(),
                );
        })
        .expect("spawn large-stack test thread")
        .join()
        .expect("large-stack parallel-subagent test thread panicked");
}

#[test]
fn spawn_parallel_agents_opts_out_of_the_global_tool_timeout() {
    // A fan-out of N long sub-agents must not be hard-killed at the single-tool
    // wall-clock default (120s): that truncates every worker and bounds the whole
    // group at one worker's budget. The tool governs its own lifetime via its
    // internal max_concurrency, cancellation token, and per-sub-agent caps.
    assert_eq!(
        SpawnParallelAgentsTool::new().timeout_policy(&json!({})),
        ToolTimeout::Unbounded,
    );
}

/// An absolute or parent-escaping ownership path is rejected with the host's
/// own sentence, not the crate's typed error.
#[test]
fn invalid_ownership_paths_are_rejected_with_the_host_message() {
    for raw in ["files: /etc/passwd", "files: ../outside.rs"] {
        let definition = definition_with_tool_scope(
            "writer",
            ToolScope::Named(vec!["write_fixture".into()]),
            SandboxMode::None,
        );
        let definitions = HashMap::from([(definition.id.clone(), definition)]);
        let parent = parent_admitting(&["writer"], write_fixture_tools());

        let preflight = prepare_spawn_parallel_tasks_from_defs(
            vec![dispatch_task("writer", Some(raw), None)],
            &definitions,
            &parent,
        );

        match &preflight[0] {
            SpawnParallelTaskPreflight::Rejected(rejection) => {
                assert_eq!(rejection.kind, ParallelTaskRejectionKind::RequiresIsolation);
                assert!(
                    rejection.error.contains("must be a relative file path"),
                    "ownership rejection wording changed for {raw}: {}",
                    rejection.error
                );
            }
            SpawnParallelTaskPreflight::Prepared(_) => {
                panic!("an out-of-workspace ownership path must be rejected: {raw}")
            }
        }
    }
}

/// Pins the write-safety dispatch decision for a mixed batch.
///
/// This is the assertion that must not move: a regression here does not fail a
/// run, it lets two edit-capable workers into one checkout at the same time and
/// returns a plausible result built on a torn tree. The sequence below —
/// worktree-isolated writer parallel, shared read-only agent parallel, shared
/// writer with disjoint ownership serialized, shared writer claiming an
/// already-claimed path rejected — is the contract.
#[test]
fn mixed_batch_dispatch_modes_and_claim_conflicts_are_stable() {
    let isolated_writer = definition_with_tool_scope(
        "isolated_writer",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let reader = definition_with_tool_scope(
        "reader",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::ReadOnly,
    );
    let writer = definition_with_tool_scope(
        "writer",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let clasher = definition_with_tool_scope(
        "clasher",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    // Defined but deliberately omitted from the parent allowlist below, so it
    // is policy-rejected *before* any claim is admitted. An earlier rejection
    // must not advance `admitted_index`, or every later admitted task would
    // resolve to the wrong plan slot and the clasher's conflict would be
    // misattributed — this batch pins that the index stays admission-scoped.
    let outside = definition_with_tool_scope(
        "outside",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let definitions = HashMap::from([
        (isolated_writer.id.clone(), isolated_writer),
        (reader.id.clone(), reader),
        (writer.id.clone(), writer),
        (clasher.id.clone(), clasher),
        (outside.id.clone(), outside),
    ]);
    let parent = parent_admitting(
        &["isolated_writer", "reader", "writer", "clasher"],
        write_fixture_tools(),
    );

    let preflight = prepare_spawn_parallel_tasks_from_defs(
        vec![
            dispatch_task("outside", None, None),
            dispatch_task("isolated_writer", None, Some("worktree")),
            dispatch_task("reader", None, None),
            dispatch_task("writer", Some("files: src/a.rs"), None),
            dispatch_task("clasher", Some("files: src/a.rs"), None),
        ],
        &definitions,
        &parent,
    );

    // The earlier policy rejection is surfaced in place, without advancing the
    // admission index the later dispatch decisions key off.
    match &preflight[0] {
        SpawnParallelTaskPreflight::Rejected(rejection) => {
            assert_eq!(rejection.kind, ParallelTaskRejectionKind::OutsideAllowlist);
            assert_eq!(rejection.agent_id, "outside");
        }
        SpawnParallelTaskPreflight::Prepared(_) => {
            panic!("an agent outside the allowlist must be rejected")
        }
    }

    let modes: Vec<Option<WorkerDispatchMode>> = preflight
        .iter()
        .map(|entry| match entry {
            SpawnParallelTaskPreflight::Prepared(prepared) => Some(prepared.dispatch_mode()),
            SpawnParallelTaskPreflight::Rejected(_) => None,
        })
        .collect();

    assert_eq!(
        modes,
        vec![
            None,
            Some(WorkerDispatchMode::Parallel),
            Some(WorkerDispatchMode::Parallel),
            Some(WorkerDispatchMode::SerialSharedWorkspaceWrite),
            None,
        ],
        "write-safety dispatch decision changed after an earlier rejection"
    );

    // The rejection is the *later* claimant; the earlier one keeps its claim.
    // Index 4 (not 3) because the leading `outside` rejection did not consume
    // an admission slot.
    match &preflight[4] {
        SpawnParallelTaskPreflight::Rejected(rejection) => {
            assert_eq!(rejection.kind, ParallelTaskRejectionKind::RequiresIsolation);
            assert_eq!(rejection.agent_id, "clasher");
            assert!(
                rejection.error.contains("src/a.rs"),
                "rejection must name the contended path: {}",
                rejection.error
            );
        }
        SpawnParallelTaskPreflight::Prepared(_) => {
            panic!("a worker claiming an already-claimed path must be rejected")
        }
    }
}

/// A shared-workspace writer whose ownership paths are disjoint from every
/// earlier claim is admitted, and serialized rather than run in parallel.
#[test]
fn disjoint_ownership_admits_both_writers_serially() {
    let first = definition_with_tool_scope(
        "first",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let second = definition_with_tool_scope(
        "second",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let definitions = HashMap::from([(first.id.clone(), first), (second.id.clone(), second)]);
    let parent = parent_admitting(&["first", "second"], write_fixture_tools());

    let preflight = prepare_spawn_parallel_tasks_from_defs(
        vec![
            dispatch_task("first", Some("files: src/a.rs"), None),
            dispatch_task("second", Some("files: src/b.rs"), None),
        ],
        &definitions,
        &parent,
    );

    let modes: Vec<Option<WorkerDispatchMode>> = preflight
        .iter()
        .map(|entry| match entry {
            SpawnParallelTaskPreflight::Prepared(prepared) => Some(prepared.dispatch_mode()),
            SpawnParallelTaskPreflight::Rejected(_) => None,
        })
        .collect();

    assert_eq!(
        modes,
        vec![
            Some(WorkerDispatchMode::SerialSharedWorkspaceWrite),
            Some(WorkerDispatchMode::SerialSharedWorkspaceWrite),
        ]
    );
}

/// Directory-level ownership contains the files beneath it, so a worker
/// claiming `src` collides with one claiming `src/a.rs`.
#[test]
fn directory_ownership_contains_files_beneath_it() {
    let owner = definition_with_tool_scope(
        "owner",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let nested = definition_with_tool_scope(
        "nested",
        ToolScope::Named(vec!["write_fixture".into()]),
        SandboxMode::None,
    );
    let definitions = HashMap::from([(owner.id.clone(), owner), (nested.id.clone(), nested)]);
    let parent = parent_admitting(&["owner", "nested"], write_fixture_tools());

    let preflight = prepare_spawn_parallel_tasks_from_defs(
        vec![
            dispatch_task("owner", Some("files: src"), None),
            dispatch_task("nested", Some("files: src/a.rs"), None),
        ],
        &definitions,
        &parent,
    );

    // The directory-level owner keeps its claim and serializes against any
    // sibling write rather than fanning out.
    let owner = match &preflight[0] {
        SpawnParallelTaskPreflight::Prepared(prepared) => prepared,
        SpawnParallelTaskPreflight::Rejected(_) => panic!("owner must be admitted"),
    };
    assert_eq!(
        owner.dispatch_mode(),
        WorkerDispatchMode::SerialSharedWorkspaceWrite,
        "directory-level owner serializes against shared-workspace writes"
    );

    // A file beneath an already-claimed directory is rejected with the
    // rejected task's ownership claim, the rejecting agent's identity, and
    // an error naming the contended owner directory — rather than silently
    // overlapping the owner.
    match &preflight[1] {
        SpawnParallelTaskPreflight::Rejected(rejection) => {
            assert_eq!(rejection.kind, ParallelTaskRejectionKind::RequiresIsolation);
            assert_eq!(rejection.agent_id, "nested");
            assert_eq!(
                rejection.ownership.as_deref(),
                Some("files: src/a.rs"),
                "rejection must carry the rejected task's ownership claim"
            );
            assert!(
                rejection.error.contains("'src'"),
                "rejection error must name the contended owner directory: {}",
                rejection.error
            );
        }
        SpawnParallelTaskPreflight::Prepared(_) => {
            panic!("a file beneath an already-claimed directory must be rejected")
        }
    }
}
