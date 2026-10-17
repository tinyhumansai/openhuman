use super::*;
use crate::agent::subagent_host::run_subagent;
use tinyagents_harness::context::{current_run_depth, with_run_depth, RunDepth};

#[tokio::test]
async fn runner_rejects_spawn_beyond_max_depth() {
    let provider = ScriptedProvider::new(vec![text_response("should not be called")]);
    let parent = make_parent(provider.clone(), vec![]);
    let def = make_def_named_tools(&[]);

    let result = (async {
        with_run_depth(
            RunDepth {
                depth: MAX_SPAWN_DEPTH,
                max_depth: MAX_SPAWN_DEPTH,
            },
            async {
                run_subagent(
                    &def,
                    "x",
                    with_parent(parent.clone(), SubagentRunOptions::default()),
                )
                .await
            },
        )
        .await
    })
    .await;

    assert!(
        matches!(
            result,
            Err(SubagentRunError::SpawnDepthExceeded {
                attempted_depth,
                max_depth
            }) if attempted_depth == MAX_SPAWN_DEPTH + 1 && max_depth == MAX_SPAWN_DEPTH
        ),
        "got: {result:?}"
    );
    assert!(
        provider.captured.lock().is_empty(),
        "depth rejection must happen before provider dispatch"
    );
    assert_eq!(
        current_run_depth(),
        None,
        "depth task-local must not leak after rejection"
    );
}

#[tokio::test]
async fn typed_mode_model_override_pins_exact_model_for_spawn() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![]);
    let mut def = make_def_named_tools(&[]);
    def.model = ModelSpec::Inherit;

    let _ = (async {
        run_subagent(
            &def,
            "use the pinned model",
            with_parent(
                parent.clone(),
                SubagentRunOptions {
                    model_override: Some("deepseek/deepseek-r2".into()),
                    ..Default::default()
                },
            ),
        )
        .await
    })
    .await
    .expect("runner should succeed");

    let captured = provider.captured.lock();
    assert_eq!(captured[0].model, "deepseek/deepseek-r2");
}

/// #1122 — when the parent attaches a progress sink, the inner loop
/// emits `SubagentIterationStarted` for each round and a paired
/// `SubagentToolCallStarted` / `SubagentToolCallCompleted` for each
/// child tool call. The web-channel bridge translates these into the
/// `subagent_iteration_start` / `subagent_tool_call` /
/// `subagent_tool_result` socket events the parent thread renders.
#[tokio::test]
async fn typed_mode_emits_child_progress_events_when_sink_attached() {
    use crate::agent::progress::AgentProgress;

    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{\"path\":\"x\"}"),
        text_response("done"),
    ]);
    let mut parent = make_parent(provider, vec![stub("file_read")]);

    // Wire the parent's progress sink so the runner re-emits child
    // lifecycle events through the same channel a real session would
    // expose to the web bridge.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
    parent.on_progress = Some(tx);

    let def = make_def_named_tools(&["file_read"]);
    let outcome = (async {
        run_subagent(
            &def,
            "read x",
            with_parent(parent, SubagentRunOptions::default()),
        )
        .await
    })
    .await
    .expect("runner should succeed");
    assert_eq!(outcome.iterations, 2);

    // Drain everything the runner sent. The receiver's sender half is
    // dropped when `parent` falls out of scope above, so `recv` returns
    // None once the queue empties.
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }

    let iter_starts = events
        .iter()
        .filter(|e| matches!(e, AgentProgress::SubagentIterationStarted { .. }))
        .count();
    assert_eq!(iter_starts, 2, "one iteration_start per round");

    let tool_starts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentProgress::SubagentToolCallStarted {
                call_id,
                tool_name,
                iteration,
                ..
            } => Some((call_id.clone(), tool_name.clone(), *iteration)),
            _ => None,
        })
        .collect();
    assert_eq!(tool_starts.len(), 1);
    assert_eq!(tool_starts[0].1, "file_read");
    assert_eq!(tool_starts[0].2, 1);

    let tool_done: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentProgress::SubagentToolCallCompleted {
                call_id,
                success,
                iteration,
                ..
            } => Some((call_id.clone(), *success, *iteration)),
            _ => None,
        })
        .collect();
    assert_eq!(tool_done.len(), 1);
    assert_eq!(tool_done[0].0, tool_starts[0].0, "matching call_id pair");
    assert!(tool_done[0].1, "stub tool returns ok");
    assert_eq!(tool_done[0].2, 1);
}

/// A sub-agent's streamed visible text and reasoning are forwarded to the
/// parent's progress sink as `SubagentTextDelta` / `SubagentThinkingDelta`
/// events tagged with the child's `agent_id` / `task_id`, in order, and
/// the concatenated text deltas reconstruct the final assistant text. The
/// web-channel bridge turns these into `subagent_text_delta` /
/// `subagent_thinking_delta` socket events the parent thread renders live.
#[tokio::test]
async fn typed_mode_forwards_child_text_and_thinking_deltas() {
    use crate::agent::progress::AgentProgress;

    let provider = ScriptedProvider::new(vec![text_response_with_reasoning(
        "the final answer",
        "let me reason about this",
    )]);
    let mut parent = make_parent(provider, vec![]);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
    parent.on_progress = Some(tx);

    let def = make_def_named_tools(&[]);
    let outcome = (async {
        run_subagent(
            &def,
            "answer me",
            with_parent(parent, SubagentRunOptions::default()),
        )
        .await
    })
    .await
    .expect("runner should succeed");
    assert_eq!(outcome.output, "the final answer");

    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }

    let thinking: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentProgress::SubagentThinkingDelta {
                agent_id,
                task_id,
                delta,
                iteration,
            } => Some((agent_id.clone(), task_id.clone(), delta.clone(), *iteration)),
            _ => None,
        })
        .collect();
    assert_eq!(thinking.len(), 1, "one thinking delta forwarded");
    assert_eq!(thinking[0].2, "let me reason about this");
    assert_eq!(thinking[0].3, 1, "tagged with the child iteration");
    assert!(!thinking[0].1.is_empty(), "carries the child task id");

    let text: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentProgress::SubagentTextDelta {
                agent_id,
                task_id,
                delta,
                iteration,
            } => Some((agent_id.clone(), task_id.clone(), delta.clone(), *iteration)),
            _ => None,
        })
        .collect();
    assert_eq!(text.len(), 1, "one text delta forwarded");
    assert_eq!(text[0].2, "the final answer");
    assert_eq!(text[0].3, 1);
    // Same child identity on both delta kinds so the UI attributes them to
    // one subagent row.
    assert_eq!(text[0].0, thinking[0].0, "same agent_id");
    assert_eq!(text[0].1, thinking[0].1, "same task_id");

    // Ordering: the thinking delta precedes the text delta within the
    // iteration, matching the provider's emission order.
    let thinking_pos = events
        .iter()
        .position(|e| matches!(e, AgentProgress::SubagentThinkingDelta { .. }))
        .unwrap();
    let text_pos = events
        .iter()
        .position(|e| matches!(e, AgentProgress::SubagentTextDelta { .. }))
        .unwrap();
    assert!(
        thinking_pos < text_pos,
        "thinking streams before visible text"
    );
}

/// Runs without an attached sink must remain backwards compatible — the
/// runner is a no-op for child progress and the outcome is unchanged.
#[tokio::test]
async fn typed_mode_progress_emission_is_a_noop_without_sink() {
    let provider = ScriptedProvider::new(vec![text_response("done")]);
    let parent = make_parent(provider, vec![]);
    assert!(parent.on_progress.is_none());
    let def = make_def_named_tools(&[]);
    let outcome = (async {
        run_subagent(
            &def,
            "x",
            with_parent(parent.clone(), SubagentRunOptions::default()),
        )
        .await
    })
    .await
    .expect("runner should succeed");
    assert_eq!(outcome.iterations, 1);
}

// ── resolve_subagent_source ───────────────────────────────────────────

#[test]
fn resolve_subagent_source_inherit_uses_parent_source_and_model() {
    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let parent_source = crate::agent::tinyagents::TurnModelSource::from_model(parent.clone());
    let (_resolved_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Inherit,
        "test_agent",
        None,
        parent_source,
        "parent-model-x".to_string(),
        false,
        None,
        0.0,
    );
    assert_eq!(resolved_model, "parent-model-x");
}

#[test]
fn resolve_subagent_source_exact_overrides_only_model() {
    // Exact keeps the parent's provider but replaces the model name.
    // This is the explicit "I want a cheaper tier on the same backend"
    // escape hatch.
    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let (_resolved_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Exact("haiku-mini".to_string()),
        "test_agent",
        None,
        crate::agent::tinyagents::TurnModelSource::from_model(parent.clone()),
        "parent-model-x".to_string(),
        false,
        None,
        0.0,
    );
    assert_eq!(resolved_model, "haiku-mini");
}

#[test]
fn resolve_subagent_source_spawn_override_wins_over_definition_model() {
    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let (_resolved_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Exact("definition-model".to_string()),
        "test_agent",
        None,
        crate::agent::tinyagents::TurnModelSource::from_model(parent.clone()),
        "parent-model-x".to_string(),
        false,
        Some("spawn-model-y"),
        0.0,
    );
    assert_eq!(resolved_model, "spawn-model-y");
}

#[test]
fn resolve_subagent_source_config_model_wins_over_definition_model() {
    use crate::config::{Config, TeamModelConfig};

    let mut config = Config::default();
    config.teams.insert(
        "test_agent".to_string(),
        TeamModelConfig {
            lead_model: None,
            agent_model: Some("configured-agent-model".to_string()),
        },
    );

    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let (_resolved_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Exact("definition-model".to_string()),
        "test_agent",
        Some(&config),
        crate::agent::tinyagents::TurnModelSource::from_model(parent.clone()),
        "parent-model-x".to_string(),
        false,
        None,
        0.0,
    );
    assert_eq!(resolved_model, "configured-agent-model");
}

#[test]
fn resolve_subagent_source_inline_override_wins_over_config_model() {
    use crate::config::{Config, TeamModelConfig};

    let mut config = Config::default();
    config.teams.insert(
        "test_agent".to_string(),
        TeamModelConfig {
            lead_model: None,
            agent_model: Some("configured-agent-model".to_string()),
        },
    );

    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let (_resolved_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Exact("definition-model".to_string()),
        "test_agent",
        Some(&config),
        crate::agent::tinyagents::TurnModelSource::from_model(parent),
        "parent-model-x".to_string(),
        false,
        Some("inline-model"),
        0.0,
    );
    assert_eq!(resolved_model, "inline-model");
}

#[test]
fn resolve_subagent_source_agent_suffix_resolves_its_team_pin() {
    use crate::config::{Config, TeamModelConfig};

    let mut config = Config::default();
    config.teams.insert(
        "research".to_string(),
        TeamModelConfig {
            lead_model: Some("research-lead-model".to_string()),
            agent_model: Some("research-agent-model".to_string()),
        },
    );

    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let (_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Hint("agentic".to_string()),
        "research_agent",
        Some(&config),
        crate::agent::tinyagents::TurnModelSource::from_model(parent),
        "parent-model-x".to_string(),
        false,
        None,
        0.0,
    );
    assert_eq!(resolved_model, "research-agent-model");
}

#[test]
fn resolve_subagent_source_hint_with_no_config_falls_back() {
    // The async config load failed (transient I/O, missing file, etc.).
    // The Hint arm must NOT silently swallow the failure and synthesise
    // `{workload}-v1` — that's the OpenHuman-only naming that breaks
    // Anthropic/OpenAI. Fall back to the parent's known-good
    // (provider, model) instead.
    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let (_resolved_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Hint("agentic".to_string()),
        "test_agent",
        None, // no config loaded
        crate::agent::tinyagents::TurnModelSource::from_model(parent.clone()),
        "real-claude-id".to_string(),
        false,
        None,
        0.0,
    );
    assert_eq!(
        resolved_model, "real-claude-id",
        "model must be parent's current model — NOT '{{workload}}-v1'"
    );
}

#[test]
fn resolve_subagent_source_hint_with_config_routes_via_factory() {
    // The Hint arm with a real config takes the workload-factory path.
    // We don't assert the *resulting* provider identity here (the
    // factory may return a fresh OpenHuman backend or whatever
    // primary_cloud resolves to), but we DO assert the resolved model
    // is the workload's canonical managed tier — NOT `default_model`,
    // and NOT the parent's model.
    //
    // Every managed role runs on the managed default model; a retired tier
    // slug in `default_model` is not a pin and must not leak to the backend.
    use crate::config::Config;
    let mut config = Config::default();
    // Route `agentic` to the OpenHuman backend explicitly, and set a
    // distinct `default_model` so the assertion proves the role — not the
    // global default — drives the resolved tier.
    config.agentic_provider = Some("openhuman".to_string());
    config.default_model = Some("chat-v1".to_string());

    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let (_resolved_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Hint("agentic".to_string()),
        "test_agent",
        Some(&config),
        crate::agent::tinyagents::TurnModelSource::from_model(parent),
        "parent-model-ignored-on-hint".to_string(),
        false,
        None,
        0.0,
    );
    assert_eq!(
        resolved_model,
        crate::config::MODEL_MANAGED_DEFAULT,
        "Hint must resolve to the managed default model, not a retired tier \
         slug from default_model or the parent's model"
    );
}

#[test]
fn resolve_subagent_source_hint_falls_back_on_factory_error() {
    // An invalid provider string in the workload config (e.g. a typo
    // like "groq:something") makes the factory return Err. The Hint
    // arm must fall back to the parent provider rather than
    // propagating — sub-agent execution should degrade to "use what
    // the parent uses" not crash entirely.
    use crate::config::Config;
    let mut config = Config::default();
    config.agentic_provider = Some("groq:not-a-real-prefix".to_string());

    let parent: Arc<dyn ChatModel<()>> = ScriptedProvider::new(vec![]);
    let (_resolved_source, resolved_model) = super::super::resolve_subagent_source(
        &ModelSpec::Hint("agentic".to_string()),
        "test_agent",
        Some(&config),
        crate::agent::tinyagents::TurnModelSource::from_model(parent.clone()),
        "fallback-model".to_string(),
        false,
        None,
        0.0,
    );
    assert_eq!(resolved_model, "fallback-model");
}

// ── Probe regression tests (#1710 Wave 2) ──────────────────────────
//
// `user_is_signed_in_to_composio` replaces the legacy
// `parent.composio_client.is_none()` gate. The legacy probe was
// backend-only by construction: a direct-mode user with a stored API
// key but no backend session token was falsely reported as "not signed
// in" and the spawn-time integration refresh path was silently
// skipped. These tests pin the new behaviour so that regression
// can't sneak back in.

#[test]
fn direct_mode_user_with_stored_key_passes_signed_in_check() {
    use super::super::user_is_signed_in_to_composio;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Direct mode + inline API key (the `config.composio.api_key`
    // fallback path inside `resolve_composio_route` — equivalent to a
    // stored direct key as far as the probe is concerned).
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".into());
    assert!(
        user_is_signed_in_to_composio(&config),
        "direct-mode user with stored api key must be reported as signed in"
    );
}

#[test]
fn unsigned_in_user_fails_probe() {
    use super::super::user_is_signed_in_to_composio;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Default mode = backend, no session token → factory errors with
    // "no backend session". Direct fallback is unreachable because
    // mode is not "direct".
    assert!(
        !user_is_signed_in_to_composio(&config),
        "user with neither backend session nor direct key must NOT be reported as signed in"
    );
}

/// Sanity-check: a parent agent delegating to a sub-agent must complete
/// without panicking, even on a worker thread with a tight stack — this
/// is the same recursion shape that crashed the
/// `chat-harness-subagent` Playwright lane in production with
/// `thread 'tokio-rt-worker' has overflowed its stack, fatal runtime
/// error: stack overflow`.
///
/// The deep ground-truth regression catcher for this is the
/// `chat-harness-subagent.spec.ts` Playwright spec, which exercises the
/// real orchestrator → researcher dispatch end-to-end (real provider
/// stream, real config load, real tool registry). The scripted unit
/// path here has much smaller per-frame state than production, so a
/// single stack size doesn't cleanly bracket boxed-vs-unboxed — we use
/// the loose 1 MiB worker stack as a smoke check that the dispatch
/// path remains poll-bounded after refactors. See `subagent_host/
/// ops.rs` `Box::pin` callsites for the structural fix.
#[test]
fn nested_subagent_dispatch_runs_on_a_constrained_worker_stack() {
    use async_trait::async_trait;
    use std::sync::Arc;

    struct RecursiveDelegateTool {
        inner_def: AgentDefinition,
        /// The explicit parent the nested run is spawned under (filled once the
        /// parent snapshot, which itself holds this tool, exists).
        parent: Arc<std::sync::OnceLock<ParentExecutionContext>>,
    }

    #[async_trait]
    impl Tool for RecursiveDelegateTool {
        fn name(&self) -> &str {
            "delegate_inner"
        }
        fn description(&self) -> &str {
            "Dispatches a nested sub-agent — reproduces the recursive engine poll."
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{}})
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::Execute
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let parent = self
                .parent
                .get()
                .cloned()
                .expect("parent set before the run");
            let outcome = run_subagent(
                &self.inner_def,
                "inner go",
                with_parent(parent, SubagentRunOptions::default()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("nested run_subagent failed: {e}"))?;
            Ok(ToolResult::success(outcome.output))
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(1024 * 1024)
        .enable_all()
        .build()
        .expect("build constrained-stack tokio runtime");

    let outcome = runtime.block_on(async {
        // Three scripted responses, shared by outer + inner runs
        // (providers are Arc-cloned, so both pull from the same queue):
        //   [0] outer round 1: call `delegate_inner`
        //   [1] inner round 1: return final text
        //   [2] outer round 2: return final text using the tool result
        let provider = ScriptedProvider::new(vec![
            tool_response("delegate_inner", "{}"),
            text_response("inner-final"),
            text_response("outer-final: inner-final"),
        ]);

        let inner_def = make_def_named_tools(&[]);
        let parent_slot = Arc::new(std::sync::OnceLock::new());
        let delegate_tool: Box<dyn Tool> = Box::new(RecursiveDelegateTool {
            inner_def,
            parent: parent_slot.clone(),
        });
        let parent = make_parent(
            Arc::clone(&(provider.clone() as Arc<dyn ChatModel<()>>)),
            vec![delegate_tool],
        );
        let _ = parent_slot.set(parent.clone());
        let outer_def = make_def_named_tools(&["delegate_inner"]);

        (async {
            run_subagent(
                &outer_def,
                "outer go",
                with_parent(parent.clone(), SubagentRunOptions::default()),
            )
            .await
        })
        .await
    });

    let outcome = outcome.expect(
        "nested run_subagent must complete on a 1 MiB worker stack — \
         a stack overflow here means the recursion boundary in \
         `run_typed_mode` regressed (see the `Box::pin` callsites around \
         `run_typed_mode` and the child's tinyagents drive future).",
    );
    assert!(
        outcome.output.contains("inner-final"),
        "outer should fold the inner sub-agent's result into its final \
         answer, got: {}",
        outcome.output
    );
}
