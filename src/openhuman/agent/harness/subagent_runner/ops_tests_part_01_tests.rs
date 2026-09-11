use super::*;

#[test]
fn lazy_resolver_tolerates_near_miss_slugs() {
    use crate::openhuman::agent::context::prompt::ConnectedIntegrationTool;
    let mk = |name: &str| ConnectedIntegrationTool {
        name: name.into(),
        description: "d".into(),
        parameters: None,
    };
    let resolver = LazyToolkitResolver {
        config: std::sync::Arc::new(crate::openhuman::config::Config::default()),
        actions: vec![mk("GOOGLESLIDES_BATCH_UPDATE"), mk("GMAIL_LIST_MESSAGES")],
        resolved: std::sync::Mutex::default(),
    };
    // Exact, case-insensitive, and separator/prefix drift all resolve
    // (bug-report-2026-05-26 A2).
    assert!(resolver.resolve("GMAIL_LIST_MESSAGES").is_some());
    assert!(resolver.resolve("gmail_list_messages").is_some());
    assert!(resolver.resolve("googleslides_batch_update").is_some());
    // A fabricated slug stays unresolved → routed to the "available tools"
    // error so the model self-corrects, not silently mis-dispatched.
    assert!(resolver.resolve("GMAIL_GET_LAST_3_MESSAGES").is_none());
}

#[test]
fn normalize_slug_collapses_separators_and_case() {
    assert_eq!(
        normalize_slug("GOOGLESLIDES_BATCH_UPDATE"),
        "googleslidesbatchupdate"
    );
    assert_eq!(
        normalize_slug("googleslides_batch_update"),
        "googleslidesbatchupdate"
    );
    assert_ne!(
        normalize_slug("GMAIL_GET_LAST_3_MESSAGES"),
        normalize_slug("GMAIL_LIST_MESSAGES")
    );
}

#[test]
fn filter_named_scope_keeps_only_named() {
    let parent: Vec<Box<dyn Tool>> = vec![stub("alpha"), stub("beta"), stub("gamma")];
    let def = make_def_named_tools(&["alpha", "gamma"]);
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, None);
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["alpha", "gamma"]);
}

#[test]
fn filter_wildcard_includes_all_minus_disallowed() {
    let parent: Vec<Box<dyn Tool>> = vec![stub("alpha"), stub("beta"), stub("gamma")];
    let mut def = make_def_named_tools(&[]);
    def.tools = ToolScope::Wildcard;
    def.disallowed_tools = vec!["beta".into()];
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, None);
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["alpha", "gamma"]);
}

#[test]
fn filter_wildcard_honours_disallowed_prefix_entries() {
    let parent: Vec<Box<dyn Tool>> = vec![
        stub("alpha"),
        stub("vendor_registry_register"),
        stub("vendor_marketplace_buy_identity"),
        stub("gamma"),
    ];
    let mut def = make_def_named_tools(&[]);
    def.tools = ToolScope::Wildcard;
    def.disallowed_tools = vec!["vendor_*".into()];
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, None);
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["alpha", "gamma"]);
}

#[test]
fn filter_skill_filter_restricts_to_prefix() {
    let parent: Vec<Box<dyn Tool>> = vec![
        stub("notion__search"),
        stub("notion__read"),
        stub("gmail__send"),
        stub("file_read"),
    ];
    let mut def = make_def_named_tools(&[]);
    def.tools = ToolScope::Wildcard;
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, Some("notion"));
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["notion__search", "notion__read"]);
}

#[test]
fn filter_skill_filter_combined_with_named_scope() {
    // Named scope intersects with skill_filter — only tools that
    // appear in the named list AND match the prefix survive.
    let parent: Vec<Box<dyn Tool>> = vec![
        stub("notion__search"),
        stub("notion__read"),
        stub("gmail__send"),
    ];
    let def = make_def_named_tools(&["notion__search", "gmail__send"]);
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, Some("notion"));
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["notion__search"]);
}

#[test]
fn subagent_mode_as_str_roundtrip() {
    assert_eq!(SubagentMode::Typed.as_str(), "typed");
}

#[test]
fn append_subagent_role_contract_adds_role_and_brevity_rules() {
    let rendered = append_subagent_role_contract("base prompt".to_string(), "researcher");
    assert!(rendered.contains("## Sub-agent Role Contract"));
    assert!(rendered.contains("You are a sub-agent working for a parent OpenHuman agent"));
    assert!(rendered.contains("Keep your final response concise and synthesis-ready"));
    assert!(rendered.contains("## Sub-agent Result Contract"));
    assert!(rendered.contains("Evidence used"));
    assert!(rendered.contains("Do not include facts in Answer that are not supported"));
    assert!(rendered.contains("truncated, partial, or too large"));
}

#[test]
fn append_subagent_role_contract_is_idempotent() {
    let once = append_subagent_role_contract("base prompt".to_string(), "researcher");
    let twice = append_subagent_role_contract(once.clone(), "researcher");
    assert_eq!(once, twice, "contract suffix should only appear once");
}

#[tokio::test]
async fn typed_mode_injects_current_date_and_time_into_user_message() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let def = make_def_named_tools(&[]);

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "the actual task prompt",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await
    .unwrap();

    let captured = provider.captured.lock();
    let user_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message should be present");
    assert!(
        user_msg.content.contains("Current Date & Time:"),
        "subagent user message must include current date/time context, got: {}",
        user_msg.content
    );
}

#[tokio::test]
async fn typed_mode_system_prompt_includes_subagent_role_contract() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let def = make_def_named_tools(&[]);

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "the actual task prompt",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await
    .unwrap();

    let captured = provider.captured.lock();
    let system_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "system")
        .expect("system message should be present");
    assert!(system_msg.content.contains("## Sub-agent Role Contract"));
    assert!(system_msg
        .content
        .contains("You are a sub-agent working for a parent OpenHuman agent"));
    assert!(system_msg
        .content
        .contains("Keep your final response concise and synthesis-ready"));
}

#[tokio::test]
async fn typed_mode_returns_text_through_runner() {
    let provider = ScriptedProvider::new(vec![text_response("X is Y")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let def = make_def_named_tools(&[]);

    let outcome = with_parent_context(parent, async {
        run_subagent(
            &def,
            "summarise X",
            SubagentRunOptions {
                workspace_descriptor: None,
                skill_filter_override: None,
                toolkit_override: None,
                context: None,
                model_override: None,
                task_id: Some("t1".into()),
                worker_thread_id: None,
                initial_history: None,
                checkpoint_dir: None,
                worktree_action_dir: None,
                run_queue: None,
            },
        )
        .await
    })
    .await
    .expect("runner should succeed");

    assert_eq!(outcome.output, "X is Y");
    assert_eq!(outcome.iterations, 1);
    assert_eq!(outcome.mode, SubagentMode::Typed);
    assert_eq!(outcome.task_id, "t1");
}

#[tokio::test]
async fn capped_no_progress_subagent_returns_incomplete_status() {
    use crate::openhuman::agent::harness::subagent_runner::SubagentRunStatus;
    // A sub-agent that keeps issuing tool calls without ever producing a final
    // answer makes no progress and is halted at its model-call cap. The runner
    // summarizes the run-so-far into a resumable checkpoint and reports
    // `Incomplete` (NOT `Completed`) so the orchestrator relays the partial
    // handback instead of mistaking the no-progress summary for a result (#4096).
    //
    // The legacy repeat-identical-call circuit-breaker `Halted` distinction
    // folded into this cap handling during the tinyagents migration (#4249) —
    // see `run_subagent`'s status mapping. With `max_iterations = 2` the two
    // scripted tool calls exhaust the budget; the checkpoint summary call then
    // draws the deterministic "reached my tool-call limit" digest.
    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{\"path\":\"a\"}"),
        tool_response("file_read", "{\"path\":\"a\"}"),
    ]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let mut def = make_def_named_tools(&["file_read"]);
    def.max_iterations = 2;

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "read the file", SubagentRunOptions::default()).await
    })
    .await
    .expect("a cap halt is still Ok (not Err)");

    match outcome.status {
        SubagentRunStatus::Incomplete { reason } => assert!(
            reason.contains("limit") || reason.contains("tool-call"),
            "incomplete reason should describe the cap stop: {reason}"
        ),
        other => panic!("expected Incomplete, got {other:?}"),
    }
    assert!(
        outcome.output.contains("tool-call limit"),
        "the partial output should carry the cap-hit checkpoint summary: {}",
        outcome.output
    );
}

#[tokio::test]
async fn run_queue_steer_lands_in_subagent_history() {
    // End-to-end proof that flipping the subagent loop's run-queue arg from
    // `None` to `Some(queue)` wires steering all the way through: a message
    // pushed to the queue before the run is drained by the steering forwarder
    // in the child's turn (`run_turn_via_tinyagents_shared`) and appears as a
    // `[User steering message]:` user turn in the exact request sent to the
    // provider. This is the mechanism behind the `steer_subagent` tool.
    let provider = ScriptedProvider::new(vec![text_response("acknowledged")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let def = make_def_named_tools(&[]);

    let run_queue = RunQueue::new();
    run_queue
        .push(QueuedMessage {
            text: "switch focus to memory safety".into(),
            mode: QueueMode::Steer,
            client_id: "steer_subagent".into(),
            thread_id: "t-steer".into(),
            queued_at_ms: 0,
            model_override: None,
            temperature: None,
            profile_id: None,
            locale: None,
        })
        .await;

    let outcome = with_parent_context(parent, async {
        run_subagent(
            &def,
            "investigate the bug",
            SubagentRunOptions {
                task_id: Some("t-steer".into()),
                run_queue: Some(run_queue),
                ..Default::default()
            },
        )
        .await
    })
    .await
    .expect("runner should succeed");

    assert_eq!(outcome.output, "acknowledged");

    let captured = provider.captured.lock();
    let steered = captured[0]
        .messages
        .iter()
        .any(|m| m.role == "user" && m.content.contains("switch focus to memory safety"));
    assert!(
        steered,
        "steer message should be injected into the sub-agent's first request, got: {:?}",
        captured[0]
            .messages
            .iter()
            .map(|m| (&m.role, &m.content))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn typed_mode_no_memory_context_in_user_message() {
    // Verifies that sub-agents skip memory loading entirely: the
    // user message sent to the provider does NOT contain
    // `[Memory context]`.
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let def = make_def_named_tools(&[]);

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "the actual task prompt",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await
    .unwrap();

    let captured = provider.captured.lock();
    assert_eq!(captured.len(), 1);
    let user_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message should be present");
    assert!(
        !user_msg.content.contains("[Memory context]"),
        "subagent user message must not include memory recall section, got: {}",
        user_msg.content
    );
    assert!(user_msg.content.contains("the actual task prompt"));
}

#[tokio::test]
async fn typed_mode_includes_memory_context_when_definition_allows_it() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let mut parent = make_parent(provider.clone(), vec![stub("file_read")]);
    parent.memory_context = Arc::new(Some(
        "[Memory context]\n- prior fact: branch X failed\n".into(),
    ));
    let mut def = make_def_named_tools(&[]);
    def.omit_memory_context = false;

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "the actual task prompt",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await
    .unwrap();

    let captured = provider.captured.lock();
    let user_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message should be present");
    assert!(user_msg.content.contains("[Memory context]"));
    assert!(user_msg.content.contains("branch X failed"));
}

#[tokio::test]
async fn typed_mode_filters_tools_by_skill_filter() {
    // Parent has tools spanning notion__*, gmail__*, and a generic
    // file_read; spawn the runner with skill_filter override "notion"
    // and assert that only the notion tools end up in the request.
    let provider = ScriptedProvider::new(vec![text_response("done")]);
    let parent = make_parent(
        provider.clone(),
        vec![
            stub("notion__search"),
            stub("notion__read"),
            stub("gmail__send"),
            stub("file_read"),
        ],
    );
    // Wildcard scope so skill_filter is the only restrictor.
    let mut def = make_def_named_tools(&[]);
    def.tools = ToolScope::Wildcard;

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "lookup",
            SubagentRunOptions {
                workspace_descriptor: None,
                skill_filter_override: Some("notion".into()),
                toolkit_override: None,
                context: None,
                model_override: None,
                task_id: None,
                worker_thread_id: None,
                initial_history: None,
                checkpoint_dir: None,
                worktree_action_dir: None,
                run_queue: None,
            },
        )
        .await
    })
    .await
    .unwrap();

    // The narrow system prompt should mention the notion tools by
    // name and NOT mention gmail/file_read.
    let captured = provider.captured.lock();
    let system_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "system")
        .expect("system message present");
    assert!(system_msg.content.contains("notion__search"));
    assert!(system_msg.content.contains("notion__read"));
    assert!(
        !system_msg.content.contains("gmail__send"),
        "skill_filter should have excluded gmail__send"
    );
    assert!(
        !system_msg.content.contains("file_read"),
        "skill_filter should have excluded file_read"
    );
}

#[tokio::test]
async fn typed_mode_executes_one_tool_then_returns() {
    // Two-round script: round 1 returns a tool call, round 2 returns
    // the final text. Verifies the inner tool-call loop wires up the
    // tool result into history correctly.
    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{\"path\":\"x\"}"),
        text_response("the file contents say hello"),
    ]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    // Allow the runner to call file_read.
    let def = make_def_named_tools(&["file_read"]);

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "read x", SubagentRunOptions::default()).await
    })
    .await
    .expect("runner should succeed");

    assert!(outcome.output.contains("hello"));
    assert_eq!(outcome.iterations, 2);
    // Second request should include the role=tool message produced
    // by the runner from StubTool's "ok" output.
    let captured = provider.captured.lock();
    assert_eq!(captured.len(), 2);
    let second_call_messages = &captured[1].messages;
    let has_tool_msg = second_call_messages.iter().any(|m| m.role == "tool");
    assert!(
        has_tool_msg,
        "second provider call should include role=tool message"
    );
}

#[tokio::test]
async fn typed_mode_blocks_unallowed_tool_calls() {
    // Provider tries to call a tool that's not in the allowlist.
    // Runner should surface an error tool result and the next
    // iteration should be able to recover.
    let provider = ScriptedProvider::new(vec![
        tool_response("forbidden_tool", "{}"),
        text_response("oops, I'll try something else"),
    ]);
    let parent = make_parent(
        provider.clone(),
        vec![stub("file_read"), stub("forbidden_tool")],
    );
    // Definition only allows file_read.
    let def = make_def_named_tools(&["file_read"]);

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "do thing", SubagentRunOptions::default()).await
    })
    .await
    .expect("runner should succeed");

    assert!(outcome.output.contains("oops"));
    let captured = provider.captured.lock();
    let second_call_messages = &captured[1].messages;
    let tool_msg = second_call_messages
        .iter()
        .find(|m| m.role == "tool")
        .expect("tool result message should be present");
    // A tool outside the allowlist is never registered on the sub-agent
    // harness, so a call to it flows through the tinyagents
    // `UnknownToolPolicy::ReturnToolError` path (issue #4249): the runner
    // injects a recoverable `unknown tool `forbidden_tool` …` result (naming the
    // blocked tool and listing the valid ones) instead of executing it, and the
    // next iteration recovers. The security guarantee — the disallowed tool does
    // NOT run — is preserved; only the message wording changed from the legacy
    // "not available".
    assert!(
        tool_msg.content.contains("unknown tool") && tool_msg.content.contains("forbidden_tool"),
        "blocked tool should produce a recoverable unknown-tool error naming it: {:?}",
        tool_msg.content
    );
}

#[tokio::test]
async fn runner_errors_outside_parent_context() {
    let def = make_def_named_tools(&[]);
    let result = run_subagent(&def, "x", SubagentRunOptions::default()).await;
    assert!(matches!(result, Err(SubagentRunError::NoParentContext)));
}

#[tokio::test]
async fn subagent_emits_checkpoint_at_iteration_cap_instead_of_erroring() {
    // A sub-agent that keeps calling tools and never finishes must hit its
    // cap and return a graceful partial-progress checkpoint (Ok), not a bare
    // MaxIterationsExceeded that discards its work — so the delegating agent
    // can continue from what it got (bug-report-2026-05-26 A1, mirrors the
    // main agent). Two tool rounds (max_iterations=2), then the summarize
    // call returns prose which becomes the checkpoint.
    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{}"),
        tool_response("file_read", "{}"),
        text_response("Progress so far: read the file. Remaining: keep going."),
    ]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let mut def = make_def_named_tools(&["file_read"]);
    def.max_iterations = 2;

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "keep reading forever", SubagentRunOptions::default()).await
    })
    .await
    .expect("hitting the iteration cap should return a checkpoint, not error");

    assert!(
        outcome.output.contains("Progress so far"),
        "expected the model-written checkpoint, got: {}",
        outcome.output
    );
}

#[tokio::test]
async fn subagent_checkpoint_falls_back_to_deterministic_when_summary_empty() {
    // Same cap, but the summarize call yields nothing (response queue
    // exhausted → empty). The runner must fall back to a deterministic
    // partial-progress digest so the parent still gets a usable result
    // (bug-report-2026-05-26 A1).
    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{}"),
        tool_response("file_read", "{}"),
    ]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let mut def = make_def_named_tools(&["file_read"]);
    def.max_iterations = 2;

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "keep reading forever", SubagentRunOptions::default()).await
    })
    .await
    .expect("empty summary should fall back, not error");

    assert!(
        outcome.output.contains("tool-call limit"),
        "expected the deterministic fallback checkpoint, got: {}",
        outcome.output
    );
    assert!(
        outcome.output.contains("file_read"),
        "deterministic checkpoint should list the tool work done, got: {}",
        outcome.output
    );
}

#[tokio::test]
async fn runner_allows_spawn_at_max_depth() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![]);
    let def = make_def_named_tools(&[]);

    let outcome = with_parent_context(parent, async {
        with_spawn_depth(MAX_SPAWN_DEPTH - 1, async {
            run_subagent(&def, "x", SubagentRunOptions::default()).await
        })
        .await
    })
    .await
    .expect("runner should allow the configured maximum depth");

    assert_eq!(outcome.output, "ok");
    assert_eq!(provider.captured.lock().len(), 1);
    assert_eq!(
        current_spawn_depth(),
        0,
        "depth task-local must not leak after the run"
    );
}
