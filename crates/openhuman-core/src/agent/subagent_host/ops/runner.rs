//! Top-level sub-agent run entry points.
//!
//! [`run_subagent`] is the primary entry point for agent delegation and
//! dispatches to [`run_typed_mode`] which builds a brand-new system prompt
//! and a filtered tool list for the requested archetype, then drives provider
//! calls and tool execution until the model returns without further tool calls
//! (or the iteration budget is exhausted).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use crate::agent::harness::agent_graph::{AgentTurnRequest, AgentTurnUsage};
use crate::agent::harness::artifact_offload::{
    effective_offload_threshold, extract_artifact_paths, new_artifact_offload,
    note_artifact_handoff, offload_oversized_result, DEFAULT_OFFLOAD_THRESHOLD_BYTES,
    HANDOFF_STAGE_RECORDED,
};
use crate::agent::harness::definition::{
    validate_tier_transition, AgentDefinition, AgentDefinitionRegistry, AgentTier, IterationPolicy,
    PromptSource, SandboxMode as AgentSandboxMode,
};
use crate::agent::harness::fork_context::ParentExecutionContext;
use crate::agent::harness::{
    spawn_depth_context::current_spawn_depth, with_current_sandbox_mode, with_spawn_depth,
    MAX_SPAWN_DEPTH,
};
use crate::agent::prompts::{
    render_subagent_system_prompt_with_format, PromptContext, PromptTool, SubagentRenderOptions,
};
use crate::agent::subagent_host::subagent_iter_cap_with_autonomous_lift;
use crate::agent::subagent_host::tool_prep::{
    filter_tool_indices, is_subagent_spawn_tool, load_prompt_source, subagent_prompt_protocol,
};
use crate::agent::subagent_host::types::{
    SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome,
};
use crate::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS;
use tinytools::{SandboxMode as TinyagentsSandboxMode, ToolSpec, WorkspaceDescriptor};
use tinytools_std::file_state::with_file_state_agent_id;

use super::prompt::{
    append_artifact_offload_contract, append_subagent_role_contract, dedup_tool_specs_by_name,
};
use super::provider::{resolve_subagent_source, user_is_signed_in_to_composio};

/// Runtime spawn-hierarchy gate decision for one delegation hop.
///
/// `parent_def` is the resolved parent agent definition (looked up from the
/// global registry by its definition id) or `None` when the parent can't be
/// resolved — e.g. a dynamically-named agent (model-council juror) or a custom
/// agent absent from the registry, or any context where the registry isn't
/// initialised. A `None` parent yields `Ok(())`: we skip rather than mask, the
/// same defensive posture the loader takes for unknown child ids.
///
/// A **worker** parent is also exempted. A worker's `subagents` list holds no
/// agent id (the loader rejects one), so any spawn it reaches at runtime is
/// one the host dispatched for it. Re-denying it here would turn valid custom
/// worker agents that use `{ skills = "*" }` into runtime failures. The
/// worker-leaf authoring rule stays enforced statically at boot, and the
/// per-parent allowlist gate blocks any other worker spawn.
///
/// For chat / reasoning parents the hop is checked against
/// [`validate_tier_transition`] (the single source of truth shared with the
/// boot loader walk); a forbidden hop is logged and becomes a
/// [`SubagentRunError::TierViolation`]. Logging lives here (rather than at the
/// call site) so the deny path is exercised by this fn's unit tests.
pub(super) fn tier_gate_decision(
    parent_def: Option<&AgentDefinition>,
    child: &AgentDefinition,
    parent_agent_id: &str,
    task_id: &str,
) -> Result<(), SubagentRunError> {
    let Some(parent_def) = parent_def else {
        return Ok(());
    };
    if parent_def.agent_tier == AgentTier::Worker {
        return Ok(());
    }
    if let Err(reason) = validate_tier_transition(parent_def.agent_tier, child.agent_tier) {
        tracing::warn!(
            parent_agent = %parent_agent_id,
            parent_tier = %parent_def.agent_tier,
            child_agent = %child.id,
            child_tier = %child.agent_tier,
            task_id = %task_id,
            "[subagent_host] blocked tier-violating delegation: {reason}"
        );
        return Err(SubagentRunError::TierViolation {
            parent_tier: parent_def.agent_tier,
            child_tier: child.agent_tier,
            reason,
        });
    }
    Ok(())
}

/// Truncate `output` in place to the definition's `max_result_chars` cap (when
/// set), appending a `[...truncated]` marker. Char-count based (not byte-length)
/// to avoid panicking on a multi-byte UTF-8 sequence at the boundary.
fn apply_max_result_chars(output: &mut String, cap: Option<usize>, agent_id: &str) {
    let Some(cap) = cap else { return };
    let original_chars = output.chars().count();
    if original_chars <= cap {
        return;
    }
    tracing::debug!(
        agent_id = %agent_id,
        original_chars,
        cap,
        "[subagent_host] truncating oversized result to max_result_chars cap"
    );
    let byte_offset = output
        .char_indices()
        .nth(cap)
        .map(|(i, _)| i)
        .unwrap_or(output.len());
    output.truncate(byte_offset);
    output.push_str("\n[...truncated]");
}

/// Run a sub-agent based on its definition and a task prompt.
///
/// This is the primary entry point for agent delegation. It performs the following:
/// 1. Generates a unique `task_id` if one wasn't provided.
/// 2. Asks the turn's
///    explicit root-turn dispatch guard
///    whether a delegation can still succeed, and refuses before spending
///    anything if it cannot (#5804).
/// 3. Reads the parent's explicit [`ParentExecutionContext`] from its run
///    carrier.
/// 4. Dispatches to `run_typed_mode`.
///
/// On success returns a [`SubagentRunOutcome`] whose `output` is the
/// final assistant text. On failure the error is suitable for stringifying
/// into a `tool_result` block — including the two dispatch refusals, whose
/// messages tell the model to summarise rather than delegate again.
pub(crate) async fn run_subagent_direct(
    definition: &AgentDefinition,
    task_prompt: &str,
    options: SubagentRunOptions,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    // Unconditionally heap-allocate the entire run_subagent body so
    // every caller doesn't have to carry this future's state inline.
    // Tools that delegate run inside the parent agent's already-deep
    // turn poll (the boxed tinyagents harness drive future in
    // `run_turn_via_tinyagents_shared`), so the parent's stack would
    // otherwise pile (parent turn state + dispatch_subagent state +
    // run_subagent's wrapper state + run_typed_mode state + child turn
    // state) onto tokio's 2 MiB worker stack and abort with "thread
    // 'tokio-rt-worker' has overflowed its stack, fatal runtime error:
    // stack overflow" — observed at `[subagent_host] dispatching
    // agent_id=<worker> ...` in the `chat-harness-subagent` Playwright
    // lane crash. The inner `Box::pin`s around `run_typed_mode` and the
    // child's tinyagents drive future further chunk the child's state so
    // a single sub-agent run can't blow the stack either.
    Box::pin(async move {
        // A nested delegate writes its terminal total to this run's isolated
        // ledger. If the outer future errors or is cancelled after that child
        // has completed, Drop promotes those finished totals before the error
        // can escape. Success marks the finalizer complete after folding the
        // same entries into this run's single terminal total.
        struct UsageFinalizer {
            context: crate::agent::tinyagents::host::OpenHumanRunContext,
            complete: bool,
        }
        impl UsageFinalizer {
            fn finish(&mut self, entry: crate::agent::tinyagents::host::SubagentUsageEntry) {
                self.context.record_completed_subagent_usage(entry);
                self.complete = true;
            }
        }
        impl Drop for UsageFinalizer {
            fn drop(&mut self) {
                if !self.complete {
                    self.context.promote_completed_descendant_usage();
                }
            }
        }
        let mut usage_finalizer = UsageFinalizer {
            context: options.run_context.clone(),
            complete: false,
        };
        let task_id = options
            .task_id
            .clone()
            .unwrap_or_else(|| format!("sub-{}", uuid::Uuid::new_v4()));

        // Turn-scoped dispatch gate (#5804) — deliberately the FIRST gate, for
        // the same reason the depth gate is synchronous and pre-dispatch: a
        // delegation we already know cannot land should cost nothing, not a
        // config load, a hook, or a provider round-trip.
        //
        // Two refusals, both evidence-based and both derived from what this
        // turn has actually observed rather than from any configured constant
        // or task shape: a graceful pause has been requested at the model-call
        // cap, or less wall-clock remains than this turn's slowest completed
        // sub-agent took. Outside a turn scope the guard is absent and this is
        // a no-op, so CLI and direct invocations are unaffected.
        match options.run_context.dispatch.as_deref().map_or(
            crate::agent::tinyagents::host::DispatchDecision::Allow,
            crate::agent::tinyagents::host::TurnDispatchState::check,
        ) {
            crate::agent::tinyagents::host::DispatchDecision::Allow => {}
            crate::agent::tinyagents::host::DispatchDecision::RefusePaused {
                completed_model_calls,
                cap,
            } => {
                tracing::info!(
                    agent_id = %definition.id,
                    task_id = %task_id,
                    completed_model_calls,
                    cap,
                    "[subagent_host] dispatch refused — turn already requested a graceful pause"
                );
                return Err(SubagentRunError::PauseRequested {
                    completed_model_calls,
                    cap,
                });
            }
            crate::agent::tinyagents::host::DispatchDecision::RefuseBudget {
                remaining_ms,
                observed_max_ms,
                observed_samples,
            } => {
                tracing::info!(
                    agent_id = %definition.id,
                    task_id = %task_id,
                    remaining_ms,
                    observed_max_ms,
                    observed_samples,
                    "[subagent_host] dispatch refused — remaining budget is shorter than this \
                     turn's slowest sub-agent"
                );
                return Err(SubagentRunError::DispatchBudgetExhausted {
                    remaining_ms,
                    observed_max_ms,
                    observed_samples,
                });
            }
        }

        let parent = options
            .run_context
            .parent
            .clone()
            .ok_or(SubagentRunError::NoParentContext)?;
        let started = Instant::now();
        // Typed callers carry their depth in the run context. Direct callers
        // are scoped by `with_spawn_depth`; honor both authorities and count
        // this child spawn exactly once.
        let attempted_depth = options
            .run_context
            .spawn_depth
            .max(current_spawn_depth().saturating_add(1));

        // Synchronous pre-dispatch projection of the single depth authority
        // (`MAX_SPAWN_DEPTH`, also fed to the crate's `RunPolicy.limits.max_depth`).
        // This surfaces `SpawnDepthExceeded` before a provider round-trip and
        // across the MCP process hop; the crate's `TinyAgentsError::SubAgentDepth`
        // maps onto this same error shape for over-deep in-process runs.
        if attempted_depth > MAX_SPAWN_DEPTH {
            tracing::warn!(
                agent_id = %definition.id,
                task_id = %task_id,
                attempted_depth,
                max_depth = MAX_SPAWN_DEPTH,
                "[subagent_host] spawn depth exceeded"
            );
            return Err(SubagentRunError::SpawnDepthExceeded {
                attempted_depth,
                max_depth: MAX_SPAWN_DEPTH,
            });
        }

        // Runtime spawn-hierarchy (tier) gate — defense-in-depth alongside the
        // depth gate above. The loader validates *declared* `subagents` pairs
        // statically at boot (`validate_tier_hierarchy`), but dynamic, custom,
        // or model-chosen spawns reach this chokepoint without ever passing
        // through that walk. Resolve the parent's tier from the registry by its
        // definition id; `tier_gate_decision` rejects (and logs) any forbidden
        // chat/reasoning hop while exempting unresolved + worker parents.
        let parent_def =
            AgentDefinitionRegistry::global().and_then(|reg| reg.get(&parent.agent_definition_id));
        tier_gate_decision(parent_def, definition, &parent.agent_definition_id, &task_id)?;

        // Configured `subagentStart` hooks — the last gate before a spawn costs
        // anything. Placed after the tier gate so a spawn the graph already
        // forbids never reaches a user script, and before config load so a
        // denied spawn has no side effects at all.
        if let Err(reason) = crate::hooks::ops::subagent_starting(
            tinyagents_runtime::command_hooks::context::TurnIdentity {
                conversation_id: Some(parent.session_id.clone()),
                session_id: Some(parent.session_id.clone()),
                agent_id: Some(parent.agent_definition_id.clone()),
                ..Default::default()
            },
            &definition.id,
            task_prompt,
        )
        .await
        {
            // A denial reason is hook-supplied (`agent_message`/`user_message`),
            // so it can carry arbitrary content. Log a fixed sentence with the
            // task identity; the caller still surfaces the detailed reason back
            // to the model through `SubagentRunError::HookDenied`.
            tracing::info!(
                agent_id = %definition.id,
                task_id = %task_id,
                "[subagent_host] spawn denied by a configured hook"
            );
            return Err(SubagentRunError::HookDenied(reason));
        }

        // Load the host config exactly once for this spawn and hand it to
        // everything below. See `LoadedConfig` — `load_or_init` re-reads
        // config.toml on every call, and the runtime below is slated to move
        // into TinyAgents, where there is no config file to load.
        //
        // Deliberately placed *after* `tier_gate_decision`: `load_or_init` can
        // initialize config on first run, and a spawn the tier gate rejects
        // should not have that side effect.
        let loaded_config: LoadedConfig = Box::pin(crate::config::Config::load_or_init())
            .await
            .map(std::sync::Arc::new)
            .map_err(|e| e.to_string());

        tracing::info!(
            agent_id = %definition.id,
            task_id = %task_id,
            spawn_depth = attempted_depth,
            max_spawn_depth = MAX_SPAWN_DEPTH,
            prompt_chars = task_prompt.chars().count(),
            skill_filter = ?options.skill_filter_override.as_deref().or(definition.skill_filter.as_deref()),
            "[subagent_host] dispatching"
        );

        // Install the sub-agent's declared `sandbox_mode` as the active
        // task-local for every tool invocation inside this run.
        //
        // When the worker opted into git-worktree isolation, its isolated
        // checkout is carried on the `WorkspaceDescriptor` prepared below and
        // threaded onto the run's tinyagents `RunContext`
        // (`run_turn_via_tinyagents_shared` → `RunContext::with_workspace`).
        // Every tool call then receives it via
        // `ToolExecutionContext::from_run_context`, so acting tools (shell, git)
        // resolve their CWD to that worktree (`effective_action_dir_for_context`)
        // instead of the shared `Config.action_dir` — no task-local override
        // needed. When no descriptor is prepared (the default / non-isolated
        // path), tools fall through to `security.action_dir` and behaviour is
        // unchanged.
        let mut parent_for_subagent = parent.clone();
        parent_for_subagent.workspace_descriptor =
            workspace_descriptor_for_subagent(definition, &options, &parent, &task_id);
        if let Some(descriptor) = parent_for_subagent.workspace_descriptor.as_ref() {
            tracing::debug!(
                agent_id = %definition.id,
                task_id = %task_id,
                worktree = %descriptor.root.display(),
                policy_id = %descriptor.policy_id,
                "[subagent_host] worktree-isolated worker: descriptor will route acting-tool CWD"
            );
        }
        let run_result = Box::pin(with_spawn_depth(attempted_depth, async {
            with_file_state_agent_id(task_id.clone(), async {
                with_current_sandbox_mode(definition.sandbox_mode, async {
                    Box::pin(run_typed_mode(
                        definition,
                        task_prompt,
                        &options,
                        &parent_for_subagent,
                        &task_id,
                        &loaded_config,
                    ))
                    .await
                })
                .await
            })
            .await
        }))
        .await;

        // Feed this delegation's wall-clock into the turn's running maximum,
        // which is the only thing the budget gate above judges a later
        // dispatch against (#5804). The deterministic fast path above records
        // separately and returns, so it cannot reach here twice.
        //
        // Recorded on BOTH the success and the failure path, and before the
        // `?`: a delegation that ran for three minutes and then errored spent
        // exactly as much of the turn's budget as one that succeeded, and is
        // exactly as much evidence about what a dispatch costs. Dropping
        // failures would bias the estimate downwards, and the gate fails open,
        // so the bias would show up as the guard not firing when it should.
        //
        // Measured from the outer `started` rather than `outcome.elapsed`, so
        // the config load and the tier/hook gates are inside the figure — the
        // question the gate asks is how long a *dispatch* takes end to end,
        // not how long the child's own loop ran.
        if let Some(dispatch) = options.run_context.dispatch.as_deref() {
            dispatch.record_subagent_elapsed(started.elapsed());
        }

        let mut outcome = run_result?;

        // Commit the completed subtree before the soft, awaited artifact
        // offload. A cancellation while that filesystem work is pending must
        // not erase direct model usage or descendants that already finished.
        usage_finalizer.finish(crate::agent::tinyagents::host::SubagentUsageEntry {
            task_id: task_id.clone(),
            agent_id: definition.id.clone(),
            usage: outcome.usage,
        });

        // #3883: offload an oversized worker result to `action_dir/outputs/`
        // BEFORE the cap below truncates it, so the parent receives a path plus
        // an abstract and the full-fidelity body survives on disk instead of
        // being cut. A refused or failed offload is soft: the inline payload
        // continues on to the cap and the summarizer detour exactly as before.
        Box::pin(offload_outcome_artifacts(&mut outcome, definition, &options, &task_id)).await;

        // Truncate result to the definition's cap if set (shared with the
        // deterministic memory fast path via `apply_max_result_chars`).
        apply_max_result_chars(&mut outcome.output, definition.max_result_chars, &definition.id);

        tracing::info!(
            agent_id = %definition.id,
            task_id = %task_id,
            spawn_depth = attempted_depth,
            elapsed_ms = outcome.elapsed.as_millis() as u64,
            iterations = outcome.iterations,
            output_chars = outcome.output.chars().count(),
            "[subagent_host] completed"
        );

        let _ = started; // silence unused-warning if logging is compiled out
        Ok(outcome)
    })
    .await
}

/// Apply the filesystem-offload convention to a finished sub-agent run (#3883).
///
/// Writes an oversized result to `action_dir/outputs/` and swaps `output` for a
/// path + abstract, then records every `[artifact]` pointer the outgoing payload
/// carries — the harness-written one and any the worker authored itself by
/// following the prompt contract — onto `SubagentRunOutcome::artifact_paths`, so
/// the parent receives the paths structurally, not only as prose.
///
/// Every failure is soft. With no resolvable action root (or a refused target)
/// the outcome is left untouched and the summarizer detour plus
/// `tool_result_budget_bytes` truncation stay in charge as the fallback.
async fn offload_outcome_artifacts(
    outcome: &mut SubagentRunOutcome,
    definition: &AgentDefinition,
    options: &SubagentRunOptions,
    task_id: &str,
) {
    // Pointers the WORKER authored itself (prompt contract) are read first: the
    // harness may be about to replace `output` wholesale with its own pointer,
    // which would otherwise drop them. They also have to survive the early
    // returns below, so a run with no resolvable action root still reports the
    // paths its child wrote by hand.
    let mut paths = extract_artifact_paths(&outcome.output);

    // A worktree-isolated worker offloads into its own checkout; everyone else
    // uses the live policy's action root, which is the same root a parent's
    // relative read resolves the returned path against.
    let policy = crate::security::live_policy::current();
    let Some(action_dir) = options
        .worktree_action_dir
        .clone()
        .or_else(|| policy.as_ref().map(|p| p.action_dir.clone()))
    else {
        tracing::debug!(
            task_id = %task_id,
            agent_id = %outcome.agent_id,
            worker_authored_paths = paths.len(),
            "[artifact] no resolvable action_dir — skipping offload (summarizer/truncation backstop applies)"
        );
        outcome.artifact_paths = paths;
        note_artifact_handoff(
            HANDOFF_STAGE_RECORDED,
            &outcome.agent_id,
            task_id,
            &outcome.artifact_paths,
        );
        return;
    };

    // A read-only tier means this run may not mutate the disk at all, so the
    // harness does not persist on its behalf either. The result stays inline and
    // the summarizer / truncation backstops handle it, exactly as before #3883.
    if policy
        .as_ref()
        .is_some_and(|p| p.autonomy == crate::security::AutonomyLevel::ReadOnly)
    {
        tracing::debug!(
            task_id = %task_id,
            agent_id = %outcome.agent_id,
            "[artifact] readonly autonomy tier — skipping offload (summarizer/truncation backstop applies)"
        );
        outcome.artifact_paths = paths;
        note_artifact_handoff(
            HANDOFF_STAGE_RECORDED,
            &outcome.agent_id,
            task_id,
            &outcome.artifact_paths,
        );
        return;
    }

    // Offload at the tighter of the global default and this agent's own result
    // cap, so a definition capped below the default (flow_memory_agent at 4 000
    // chars) gets its full body on disk instead of truncated by
    // `apply_max_result_chars` immediately after.
    let threshold =
        effective_offload_threshold(DEFAULT_OFFLOAD_THRESHOLD_BYTES, definition.max_result_chars);

    // Worktree-isolated workers write inside their own checkout, but the parent
    // that receives the pointer resolves relative paths against ITS action root.
    // Render against that root so the handed-back path is one the parent can
    // actually open (relative when the worktree nests inside it, absolute when
    // it does not) rather than a bare `outputs/…` that silently misses.
    let render_root = policy
        .as_ref()
        .map(|p| p.action_dir.clone())
        .unwrap_or_else(|| action_dir.clone());
    let offload = new_artifact_offload(action_dir, policy, outcome.agent_id.clone(), task_id)
        .with_render_root(render_root);
    let (output, _artifact) =
        offload_oversized_result(std::mem::take(&mut outcome.output), &offload, threshold).await;
    outcome.output = output;

    // Merge: the harness pointer (if it fired) plus any worker-authored pointer
    // read before the swap. `extract_artifact_paths` already de-duplicates
    // within a payload; dedupe across the two sources here.
    for path in extract_artifact_paths(&outcome.output) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    outcome.artifact_paths = paths;
    note_artifact_handoff(
        HANDOFF_STAGE_RECORDED,
        &outcome.agent_id,
        task_id,
        &outcome.artifact_paths,
    );
}

fn workspace_descriptor_for_subagent(
    definition: &AgentDefinition,
    options: &SubagentRunOptions,
    parent: &ParentExecutionContext,
    task_id: &str,
) -> Option<WorkspaceDescriptor> {
    if let Some(descriptor) = options.workspace_descriptor.clone() {
        return Some(descriptor);
    }
    if let Some(descriptor) = parent.workspace_descriptor.clone() {
        return Some(descriptor);
    }
    let root = options.worktree_action_dir.clone()?;
    let sandbox = match definition.sandbox_mode {
        AgentSandboxMode::Sandboxed => TinyagentsSandboxMode::Required,
        AgentSandboxMode::None | AgentSandboxMode::ReadOnly => TinyagentsSandboxMode::Inherit,
    };
    Some(
        WorkspaceDescriptor::new(root)
            .with_policy_id(format!("openhuman.worktree:{task_id}"))
            .with_sandbox(sandbox),
    )
}

/// One spawn's snapshot of the host config, loaded once by [`run_subagent`].
///
/// `Config::load_or_init()` is **not cached** — it re-resolves the config dirs
/// and re-reads `config.toml` on every call. `run_typed_mode` needed it in six
/// places, so a single sub-agent spawn used to hit the disk six times and could
/// observe six *different* configs if the file changed mid-spawn. One snapshot
/// is both cheaper and more coherent.
///
/// The error is captured as a `String` rather than dropped to `Option` so the
/// sites that log it can say why the config was unavailable; every site
/// degrades without it.
///
/// Threading this in as a parameter (rather than loading it inside the runtime)
/// is `docs/specs/plan-agents.md` Phase 3: the sub-agent runner is slated to
/// move into TinyAgents, and a generic runtime has no config file to load.
type LoadedConfig = Result<std::sync::Arc<crate::config::Config>, String>;

// ─────────────────────────────────────────────────────────────────────────────
// Typed mode — narrow prompt, filtered tools, cheaper model
// ─────────────────────────────────────────────────────────────────────────────

/// Execute a sub-agent in "Typed" mode.
///
/// This mode builds a brand-new, minimized system prompt specifically for the
/// agent's archetype. It filters the parent's tools down to only those allowed
/// by the definition and per-spawn overrides.
async fn run_typed_mode(
    definition: &AgentDefinition,
    task_prompt: &str,
    options: &SubagentRunOptions,
    parent: &ParentExecutionContext,
    task_id: &str,
    config: &LoadedConfig,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    let started = Instant::now();

    // Resolve model source + model. See `resolve_subagent_source` for the
    // semantics of each ModelSpec variant; the helper itself is sync and
    // unit-tested, and takes the config the caller already loaded.
    let (subagent_source, model) = resolve_subagent_source(
        &definition.model,
        &definition.id,
        config.as_ref().ok().map(|c| c.as_ref()),
        parent.turn_model_source.clone(),
        parent.model_name.clone(),
        !definition.subagents.is_empty(),
        options.model_override.as_deref(),
        definition.temperature,
    );
    let temperature = definition.temperature;
    let max_output_tokens = definition
        .max_turn_output_tokens
        .unwrap_or(AGENT_TURN_MAX_OUTPUT_TOKENS);

    // ── Refresh connected-integrations at spawn time ───────────────────
    //
    // The parent session's `connected_integrations` Vec is frozen at
    // session-start. Re-fetch from the global integrations cache here.
    // The cache is invalidated by `ComposioConnectionCreatedSubscriber`
    // once the OAuth handshake reaches ACTIVE/CONNECTED, so this call
    // returns the fresh list almost for free on the warm path. Fall back
    // to the parent's frozen list when the live fetch returns empty.
    let live_integrations: Vec<crate::agent::prompts::ConnectedIntegration> = {
        let signed_in = config
            .as_ref()
            .ok()
            .map(|cfg| user_is_signed_in_to_composio(cfg))
            .unwrap_or(false);
        if !signed_in {
            parent.connected_integrations.clone()
        } else {
            match config.as_ref() {
                Ok(cfg) => {
                    use crate::integrations::composio::FetchConnectedIntegrationsStatus;
                    match crate::integrations::composio::fetch_connected_integrations_status(cfg)
                        .await
                    {
                        FetchConnectedIntegrationsStatus::Authoritative(fresh) => {
                            tracing::debug!(
                                count = fresh.len(),
                                parent_count = parent.connected_integrations.len(),
                                "[subagent_host] refreshed connected_integrations at spawn time"
                            );
                            fresh
                        }
                        FetchConnectedIntegrationsStatus::Unavailable => {
                            tracing::debug!(
                                "[subagent_host] integrations backend unavailable; falling back to parent's frozen list"
                            );
                            parent.connected_integrations.clone()
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        "[subagent_host] config load failed; falling back to parent's frozen integrations list"
                    );
                    parent.connected_integrations.clone()
                }
            }
        }
    };

    // ── Filter tools per definition + per-spawn override ───────────────
    let mut allowed_indices = filter_tool_indices(
        &parent.all_tools,
        &definition.tools,
        &definition.disallowed_tools,
        options
            .skill_filter_override
            .as_deref()
            .or(definition.skill_filter.as_deref()),
    );

    // Sub-agents must never spawn their own sub-agents. Strip `spawn_subagent`
    // and every synthesised `delegate_*` tool regardless of the archetype's
    // declared scope.
    let before = allowed_indices.len();
    allowed_indices.retain(|&i| {
        let name = parent.all_tools[i].name();
        !is_subagent_spawn_tool(name) && name != "spawn_worker_thread"
    });
    let stripped = before - allowed_indices.len();
    if stripped > 0 {
        tracing::debug!(
            agent_id = %definition.id,
            stripped,
            "[subagent_host] removed sub-agent spawn tools from sub-agent's tool surface"
        );
    }

    // ── Force-include extra_tools ──────────────────────────────────────
    if !definition.extra_tools.is_empty() {
        for (i, tool) in parent.all_tools.iter().enumerate() {
            let name = tool.name();
            if definition.extra_tools.iter().any(|n| n == name)
                && !allowed_indices.contains(&i)
                && !super::super::tool_prep::disallowed_tool_matches(
                    &definition.disallowed_tools,
                    name,
                )
                && !is_subagent_spawn_tool(name)
            {
                allowed_indices.push(i);
            }
        }
    }

    // A child may only narrow an explicit profile/channel ceiling, never widen
    // it back to `all_tools`. The parent's own role-specific prompt surface is
    // intentionally not a ceiling: coordinators delegate effectful work to
    // specialists whose tools they do not advertise directly.
    super::super::tool_prep::retain_parent_visible_tool_indices(
        &mut allowed_indices,
        &parent.all_tools,
        &parent.subagent_tool_ceiling_names,
    );

    let filtered_specs: Vec<ToolSpec> = allowed_indices
        .iter()
        .map(|&i| parent.all_tool_specs[i].as_ref().clone())
        .collect();
    let allowed_names: HashSet<String> = allowed_indices
        .iter()
        .map(|&i| parent.all_tools[i].name().to_string())
        .collect();
    let filtered_specs = crate::agent::session_host::dedup_visible_tool_specs(filtered_specs);
    let filtered_specs = dedup_tool_specs_by_name(&definition.id, filtered_specs);

    tracing::debug!(
        agent_id = %definition.id,
        model = %model,
        tool_count = allowed_names.len(),
        max_iterations = subagent_iter_cap_with_autonomous_lift(definition.effective_max_iterations()),
        iteration_policy = ?definition.iteration_policy,
        "[subagent_host:typed] resolved configuration"
    );

    // ── Build the narrow system prompt ─────────────────────────────────
    let render_options = SubagentRenderOptions::from_definition_flags(
        definition.omit_identity,
        definition.omit_safety_preamble,
    );

    let connected_integrations_for_prompt: Vec<crate::agent::prompts::ConnectedIntegration> =
        live_integrations
            .iter()
            .filter(|ci| ci.connected)
            .cloned()
            .collect();

    let prompt_tools: Vec<PromptTool<'_>> = allowed_indices
        .iter()
        .map(|&i| {
            let t = parent.all_tools[i].as_ref();
            PromptTool {
                name: std::borrow::Cow::Borrowed(t.name()),
                description: std::borrow::Cow::Borrowed(t.description()),
                parameters_schema: Some(t.parameters_schema().to_string()),
            }
        })
        .collect();
    let visible_tool_names: std::collections::HashSet<String> =
        prompt_tools.iter().map(|t| t.name.to_string()).collect();
    let (prompt_tool_call_format, dispatcher_instructions) =
        subagent_prompt_protocol(parent.tool_call_format, &filtered_specs);
    // Load AGENTS.md instruction layers once, at prompt-build time, when the
    // config gate is on. The global layer comes from the workspace dir; the
    // project layer comes from the sub-agent's `worktree_action_dir` override
    // when present (git-worktree isolation), otherwise the global config
    // `action_dir`. Loading here (not per turn) keeps the sub-agent system
    // prompt byte-stable for prefix caching.
    let agents_md = if parent.agent_config.agents_md_enabled {
        let local_dir = options
            .worktree_action_dir
            .clone()
            .or_else(|| config.as_ref().ok().map(|c| c.action_dir.clone()));
        match local_dir {
            Some(dir) => crate::agent::prompts::load_agents_md_layers(&parent.workspace_dir, &dir),
            None => {
                // No resolvable project dir — still surface the workspace layer.
                crate::agent::prompts::AgentsMdContent {
                    global: crate::agent::prompts::load_agents_md(&parent.workspace_dir),
                    local: None,
                }
            }
        }
    } else {
        tracing::debug!(
            agent_id = %definition.id,
            "[agents_md] disabled by config; skipping AGENTS.md injection for subagent"
        );
        crate::agent::prompts::AgentsMdContent::default()
    };

    let prompt_ctx = PromptContext {
        workspace_dir: &parent.workspace_dir,
        model_name: &model,
        agent_id: &definition.id,
        tools: &prompt_tools,
        workflows: &parent.workflows,
        dispatcher_instructions: &dispatcher_instructions,
        visible_tool_names: &visible_tool_names,
        tool_call_format: prompt_tool_call_format,
        connected_integrations: &connected_integrations_for_prompt,
        connected_identities_md: crate::agent::prompts::render_connected_identities(),
        user_identity: crate::security::credentials::identity::peek_credential_user_identity(),
        personality_roster: vec![],
        agents_md_global: agents_md.global.clone(),
        agents_md_local: agents_md.local.clone(),
    };

    let system_prompt = match &definition.system_prompt {
        PromptSource::Dynamic(build) => {
            build(&prompt_ctx).map_err(|e| SubagentRunError::PromptLoad {
                path: format!("<dynamic:{}>", definition.id),
                source: std::io::Error::other(e.to_string()),
            })?
        }
        PromptSource::Inline(_) | PromptSource::File { .. } => {
            let archetype_prompt_body = load_prompt_source(&definition.system_prompt, &prompt_ctx)?;
            render_subagent_system_prompt_with_format(
                &parent.workspace_dir,
                &model,
                &allowed_indices,
                &parent.all_tools,
                &[],
                &archetype_prompt_body,
                render_options,
                prompt_tool_call_format,
                &connected_integrations_for_prompt,
                agents_md.global.as_deref(),
                agents_md.local.as_deref(),
            )
        }
    };

    let system_prompt = append_subagent_role_contract(system_prompt, &definition.id);
    // #3883: only agents that actually hold a file-write tool are told to
    // offload. `visible_tool_names` is this sub-agent's real, post-filter tool
    // surface, so the contract can never advertise a tool the child cannot call.
    let system_prompt =
        append_artifact_offload_contract(system_prompt, &definition.id, &visible_tool_names);

    // ── Build the user message (with optional context prefix) ──────────
    // Shared one-line stamp (#3602) so sub-agents report time in the same
    // format as the main agent. Lives on the user message because sub-agent
    // system prompts are byte-stable for prefix caching.
    let now_str = crate::agent::prompts::current_datetime_line();

    let mut context_parts: Vec<&str> = Vec::new();
    if !definition.omit_memory_context {
        if let Some(ref mem_ctx) = *parent.memory_context {
            context_parts.push(mem_ctx);
        }
    }
    context_parts.push(&now_str);

    if let Some(ref ctx) = options.context {
        context_parts.push(ctx);
    }
    let mut history: Vec<tinyagents_session::transcript::TranscriptMessage> =
        if let Some(ref initial) = options.initial_history {
            tracing::info!(
                agent_id = %definition.id,
                task_id = %task_id,
                history_len = initial.len(),
                "[subagent_host] resuming with initial_history (checkpoint replay)"
            );
            initial.clone()
        } else {
            let user_message = if context_parts.is_empty() {
                task_prompt.to_string()
            } else {
                format!("[Context]\n{}\n\n{task_prompt}", context_parts.join("\n\n"))
            };
            vec![
                tinyagents_session::transcript::TranscriptMessage::system(system_prompt),
                tinyagents_session::transcript::TranscriptMessage::user(user_message),
            ]
        };

    // ── Run the inner tool-call loop ───────────────────────────────────
    // Resolve the sub-agent model's user-configured vision flag; defaults to
    // `false` when config can't be loaded. Combined with the provider capability
    // at the gate, this lets a flagged custom/BYOK sub-agent model forward images.
    let model_vision = config
        .as_ref()
        .ok()
        .map(|cfg| crate::inference::model_context::model_supports_vision(&model, cfg))
        .unwrap_or(false);
    tracing::debug!(
        target: "subagent_runner",
        model = %model,
        model_vision,
        "[subagent_host] resolved sub-agent model vision capability"
    );
    // Per-agent turn graph (issue #4249): `Default` runs the shared sub-agent
    // graph; `Custom` hands the assembled turn to this agent's own graph runner
    // (declared in its `graph.rs::graph()`). Every built-in agent selects
    // `Default` today — the branch is the extension point.
    use super::graph::AggregatedUsage;
    use crate::agent::harness::agent_graph::AgentGraph;
    // Resolve the child transcript stem once — `{parent_chain}__{child_session_key}`
    // — so the sub-agent's raw transcript lands in `session_raw` under a filename
    // that chains the parent session (parity with the removed observer stem).
    let child_session_key = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let unix_ts = now.as_secs();
        let nanos = now.subsec_nanos();
        let sanitized: String = definition
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let task_suffix: String = task_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(12)
            .collect();
        if task_suffix.is_empty() {
            format!("{unix_ts}_{nanos:09}_{sanitized}")
        } else {
            format!("{unix_ts}_{nanos:09}_{sanitized}_{task_suffix}")
        }
    };
    let transcript_stem = {
        let parent_chain = match parent.session_parent_prefix.as_deref() {
            Some(prefix) => format!("{}__{}", prefix, parent.session_key),
            None => parent.session_key.clone(),
        };
        format!("{parent_chain}__{child_session_key}")
    };
    let workspace_descriptor =
        workspace_descriptor_for_subagent(definition, options, parent, task_id);
    if let Some(descriptor) = &workspace_descriptor {
        tracing::debug!(
            agent_id = %definition.id,
            task_id,
            root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[subagent_host] prepared workspace descriptor for tinyagents run"
        );
    }

    let (output, iterations, agg_usage, early_exit_tool, hit_cap, breaker_halt) =
        match &definition.graph {
            AgentGraph::Default => {
                super::graph::run_subagent_via_graph(
                    subagent_source.clone(),
                    &model,
                    temperature,
                    &mut history,
                    parent.all_tools.clone(),
                    Vec::new(),
                    filtered_specs.clone(),
                    allowed_names,
                    subagent_iter_cap_with_autonomous_lift(definition.effective_max_iterations()),
                    options.run_queue.clone(),
                    parent.on_progress.clone(),
                    &definition.id,
                    task_id,
                    definition.iteration_policy == IterationPolicy::Extended,
                    options.thread_id.clone(),
                    options.run_context.clone(),
                    options.worker_thread_id.clone(),
                    parent.workspace_dir.clone(),
                    workspace_descriptor.clone(),
                    max_output_tokens,
                    model_vision,
                    &transcript_stem,
                    // Sub-agent turns record their provider label as the literal
                    // "subagent" (parity with the legacy observer's TurnObserver
                    // provenance), distinguishing delegated spend from the parent's
                    // own channel in per-thread usage reads.
                    "subagent",
                    // Agent-level TokenJuice profile → sub-agent context middleware
                    // (#4466), so sub-agent tool outputs compact like the chat path.
                    definition.effective_tokenjuice_compression(),
                    // The spawn-wide config snapshot supplies the `[context]`
                    // knobs the graph used to load for itself.
                    config.as_ref().ok().map(|c| c.as_ref()),
                )
                .await?
            }
            AgentGraph::Custom(run) => {
                let req = AgentTurnRequest {
                    turn_model_source: subagent_source.clone(),
                    model: model.clone(),
                    temperature,
                    history: std::mem::take(&mut history),
                    parent_tools: parent.all_tools.clone(),
                    dynamic_tools: Vec::new(),
                    specs: filtered_specs.clone(),
                    allowed_names,
                    max_iterations: subagent_iter_cap_with_autonomous_lift(
                        definition.effective_max_iterations(),
                    ),
                    run_queue: options.run_queue.clone(),
                    on_progress: parent.on_progress.clone(),
                    agent_id: definition.id.clone(),
                    task_id: task_id.to_string(),
                    extended_policy: definition.iteration_policy == IterationPolicy::Extended,
                    thread_id: options.thread_id.clone(),
                    run_context: options.run_context.clone(),
                    worker_thread_id: options.worker_thread_id.clone(),
                    workspace_dir: parent.workspace_dir.clone(),
                    workspace_descriptor: workspace_descriptor.clone(),
                    max_output_tokens,
                    model_vision,
                    transcript_stem: transcript_stem.clone(),
                    provider_label: "subagent".to_string(),
                    tokenjuice_compression: definition.effective_tokenjuice_compression(),
                    config: config.as_ref().ok().map(Arc::clone),
                };
                let res = run(req).await?;
                history = res.history;
                let AgentTurnUsage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    charged_amount_usd,
                } = res.usage;
                (
                    res.output,
                    res.iterations,
                    AggregatedUsage {
                        input_tokens,
                        output_tokens,
                        cached_input_tokens,
                        charged_amount_usd,
                    },
                    res.early_exit_tool,
                    res.hit_cap,
                    res.breaker_halt,
                )
            }
        };

    // Determine status: if the turn engine exited early because of
    // ask_user_clarification, checkpoint the history and return
    // AwaitingUser so the orchestrator can relay the user's answer.
    let status = if early_exit_tool.as_deref() == Some("ask_user_clarification") {
        let question = output.clone();
        let options_vec: Option<Vec<String>> = None;

        crate::agent::subagent_host::types::SubagentRunStatus::AwaitingUser {
            question,
            options: options_vec,
            // The neutral driver's persistence seam writes the one scoped
            // checkpoint after the executor returns.  Writing here would make
            // pause effects visible before lifecycle admission and duplicate
            // the durable record.
            checkpoint: None,
        }
    } else if let Some(reason) = breaker_halt {
        // The repeated-failure / repeat-progress circuit breaker halted the run
        // (#4466). It is NOT a clean finish: `output` carries the breaker's
        // root-cause summary, not a completed answer. Surface `Incomplete` with
        // the halt reason so a delegating parent relays the blocker instead of
        // treating the halted child as finished (the migrated path reported
        // `hit_cap=false` → `Completed`, hiding the halt).
        tracing::warn!(
            task_id = %task_id,
            agent_id = %definition.id,
            reason = %reason,
            "[subagent_host] child halted by circuit breaker; reporting Incomplete (#4466)"
        );
        crate::agent::subagent_host::types::SubagentRunStatus::Incomplete { reason }
    } else if hit_cap {
        // The tinyagents run stopped at the model-call cap with work still
        // pending (graph summarized a resumable checkpoint into `output`).
        // Surface it as Incomplete so the delegating agent relays the partial
        // result + blocker instead of treating the summary as a finished answer
        // or re-spinning the identical delegation (#4096).
        crate::agent::subagent_host::types::SubagentRunStatus::Incomplete {
            reason: "reached its tool-call limit before finishing".into(),
        }
    } else {
        // A clean final response. (An `ask_user_clarification` early-exit is
        // handled by the branch above.) The legacy circuit-breaker `Halted`
        // distinction folds into the tinyagents stop-hook / cap handling.
        crate::agent::subagent_host::types::SubagentRunStatus::Completed
    };

    // Surface this run's token/cost totals so the parent turn can roll them
    // into the session-level meters and the global cost tracker. The caller's
    // explicit host carrier owns the ledger; no Tokio task scope is involved.
    let mut usage = crate::agent::subagent_host::types::SubagentUsage {
        input_tokens: agg_usage.input_tokens,
        output_tokens: agg_usage.output_tokens,
        cached_input_tokens: agg_usage.cached_input_tokens,
        charged_amount_usd: agg_usage.charged_amount_usd,
    };
    // A nested child records on this run's isolated ledger. Fold those totals
    // into the completed child before writing the immediate parent's ledger so
    // the root always receives complete subtree spend without siblings sharing
    // mutable in-flight state.
    for entry in options.run_context.subagent_usage_entries() {
        usage.input_tokens = usage.input_tokens.saturating_add(entry.usage.input_tokens);
        usage.output_tokens = usage
            .output_tokens
            .saturating_add(entry.usage.output_tokens);
        usage.cached_input_tokens = usage
            .cached_input_tokens
            .saturating_add(entry.usage.cached_input_tokens);
        usage.charged_amount_usd += entry.usage.charged_amount_usd;
    }
    Ok(SubagentRunOutcome {
        task_id: task_id.to_string(),
        agent_id: definition.id.clone(),
        output,
        iterations,
        elapsed: started.elapsed(),
        mode: SubagentMode::Typed,
        status,
        final_history: history,
        usage,
        // Filled in by `run_subagent` once the offload step has had its say, so
        // both the harness-offloaded and worker-authored pointers are counted.
        artifact_paths: Vec::new(),
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalExisting,
    })
}

#[cfg(test)]
#[path = "runner_result_cap_tests.rs"]
mod result_cap_tests;
