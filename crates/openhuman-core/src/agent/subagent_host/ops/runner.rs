//! Top-level sub-agent run entry points.
//!
//! The registered dispatcher reaches this adapter, which dispatches to
//! [`run_typed_mode`] to build a brand-new system prompt
//! and a filtered tool list for the requested archetype, then drives provider
//! calls and tool execution until the model returns without further tool calls
//! (or the iteration budget is exhausted).

use std::collections::HashSet;
use std::time::Instant;

use crate::agent::prompts::render_subagent_system_prompt_with_format;
use crate::agent::prompts::{PromptContext, SubagentRenderOptions};
use crate::agent::subagent_host::definition_iter_cap;
use crate::agent::subagent_host::tool_prep::{
    dedup_owned_tool_specs, filter_definition_tool_indices, subagent_prompt_protocol,
};
use crate::agent::subagent_host::types::{
    SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome,
};
use crate::agent::subagent_host::ParentExecutionContext;
use crate::agent::tinyagents::budget::MAX_SPAWN_DEPTH;
use crate::agent_registry::host_definition::{AgentDefinition, AgentDefinitionRegistry};
use tinyagents_definition::{validate_tier_transition, AgentTier, IterationPolicy, PromptSource};
use tinyagents_harness::context::{current_run_depth, with_run_depth, RunConfig, RunDepth};
use tinyagents_harness::turn_policy::AGENT_TURN_MAX_OUTPUT_TOKENS;
use tinyagents_harness_tools::fs::file_state::with_file_state_agent_id;
use tinytools::{Tool, ToolSpec};

use super::prompt::compose_subagent_system_prompt;
use super::provider::resolve_subagent_source;

#[path = "runner_prompt.rs"]
mod runner_prompt;
use runner_prompt::load_typed_prompt_source;

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

/// Execute a prepared sub-agent based on its definition and a task prompt.
///
/// Creates a task id, checks the root-turn dispatch budget, reads the parent's
/// [`ParentExecutionContext`], then dispatches to `run_typed_mode`.
///
/// On success returns a [`SubagentRunOutcome`] whose `output` is the
/// final assistant text. On failure the error is suitable for stringifying
/// into a `tool_result` block — including the two dispatch refusals, whose
/// messages direct the model to summarize rather than delegate again.
pub(crate) async fn run_subagent_direct_with_native_context(
    definition: &AgentDefinition,
    task_prompt: &str,
    mut options: SubagentRunOptions,
    native: crate::agent::tinyagents::NativeSubagentRunContext,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    // Unconditionally heap-allocate the entire child execution body so
    // every caller doesn't have to carry this future's state inline.
    // Tools that delegate run inside the parent agent's already-deep
    // turn poll (the boxed tinyagents harness drive future in
    // `TurnSpec::run`), so the parent's stack would
    // otherwise pile (parent turn state + dispatch_subagent state +
    // dispatcher state + run_typed_mode state + child turn
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
        // The host carries an already-derived child depth; native and MCP
        // callers expose their parent depth through TinyAgents' context.
        let parent_depth = options.run_context.spawn_depth.saturating_sub(1)
            .max(current_run_depth().map_or(0, |facts| facts.depth));
        let max_depth = crate::agent::tinyagents::budget::depth(&options.run_context);
        let attempted_depth = RunConfig::checked_child_depth(parent_depth, max_depth)
            .map_err(|_| SubagentRunError::SpawnDepthExceeded {
                attempted_depth: parent_depth.saturating_add(1),
                max_depth,
            })?;
        let nested_spawn_depth_available =
            RunConfig::checked_child_depth(attempted_depth, max_depth).is_ok();
        options.run_context.spawn_depth = attempted_depth;
        // Acts on memory as a child of its parent: same team, own definition.
        let acting = options.run_context.memory_identity.take().unwrap_or_default();
        options.run_context.memory_identity = Some(acting.child(&definition.id));
        // Runtime spawn-hierarchy (tier) gate — defense-in-depth alongside the
        // depth gate above. The loader validates *declared* `subagents` pairs
        // statically at boot (`validate_tier_hierarchy`), but dynamic, custom,
        // or model-chosen spawns reach this chokepoint without ever passing
        // through that walk. Resolve the parent's tier from the registry by its
        // definition id; `tier_gate_decision` rejects (and logs) any forbidden
        // chat/reasoning hop while exempting unresolved + worker parents.
        let parent_registry = AgentDefinitionRegistry::current();
        let parent_def = parent_registry
            .as_deref()
            .and_then(|reg| reg.get(&parent.agent_definition_id));
        tier_gate_decision(parent_def, definition, &parent.agent_definition_id, &task_id)?;
        // Configured `subagentStart` hooks — the last gate before a spawn costs
        // anything. Placed after the tier gate so a spawn the graph already
        // forbids never reaches a user script, and before config load so a
        // denied spawn has no side effects at all.
        if let Err(reason) = crate::hooks::ops::subagent_starting(
            tinyagents_hooks::command::context::TurnIdentity {
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

        // Loaded once per spawn (see `LoadedConfig`), after the tier gate so a
        // rejected spawn never initialises config on first run.
        let loaded_config: LoadedConfig = Box::pin(crate::config::ops::load_current_or_init(&options.run_context.dispatch_ctx()))
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

        // The child carrier projects the definition's sandbox mode to every
        // tool through HostOperationContext.
        //
        let mut parent_for_subagent = parent.clone();
        // The direct-child list belongs to the agent now being run. It is not
        // the originating parent's list: each definition's allowlist governs
        // only that actor's own children.
        parent_for_subagent.agent_definition_id = definition.id.clone();
        parent_for_subagent.allowed_subagent_ids =
            definition.allowed_subagent_ids().into_iter().collect();
        if let Some(descriptor) = options.workspace_descriptor.clone() {
            parent_for_subagent.workspace_descriptor = Some(descriptor);
        }
        let run_result = Box::pin(with_run_depth(RunDepth { depth: attempted_depth, max_depth }, async {
            with_file_state_agent_id(task_id.clone(), async {
                Box::pin(run_typed_mode(
                    definition,
                    task_prompt,
                    &options,
                    &parent_for_subagent,
                    &task_id,
                    &loaded_config,
                    native,
                    nested_spawn_depth_available,
                ))
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

        // The owner policy makes the generic byte-overflow decision and calls
        // this host's authorized artifact store. Project its opaque reference
        // into the product pointer before applying the independent character
        // cap, preserving the existing full-output-first behavior.
        if let Some(error) = super::runner_artifacts::apply_subagent_result_policies(
            &mut outcome,
            definition,
            &options,
            &task_id,
        )
        .await
        {
            tracing::debug!(task_id = %task_id, error = %error, "[artifact] sub-agent result policy fell back to inline output");
        }

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

/// One spawn's snapshot of the host config, loaded once by the host adapter.
///
/// `Config::load_or_init()` is **not cached** — it re-resolves the config dirs
/// and re-reads `config.toml` on every call. `run_typed_mode` needed it in six
/// places, so a single sub-agent spawn used to hit the disk six times and could
/// observe six *different* configs if the file changed mid-spawn.
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
    native: crate::agent::tinyagents::NativeSubagentRunContext,
    nested_spawn_depth_available: bool,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    let started = Instant::now();

    // Resolve model source + model. See `resolve_subagent_source` for the
    // semantics of each ModelSpec variant; the helper resolves any credential
    // snapshot before returning and takes the config already loaded here.
    let (subagent_source, model) = resolve_subagent_source(
        &definition.model,
        &definition.id,
        config.as_ref().ok().map(|c| c.as_ref()),
        parent.turn_model_source.clone(),
        parent.model_name.clone(),
        !definition.subagents.is_empty(),
        options.model_override.as_deref(),
        definition.temperature,
    )
    .await;
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
    let live_integrations: Vec<crate::integrations::ConnectedIntegration> = {
        let signed_in = match config.as_ref() {
            Ok(cfg) => crate::integrations::composio::user_is_signed_in_to_composio(cfg).await,
            Err(_) => false,
        };
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
    let filtered_tools = filter_definition_tool_indices(
        &parent.all_tools,
        definition,
        options
            .skill_filter_override
            .as_deref()
            .or(definition.skill_filter.as_deref()),
        nested_spawn_depth_available,
        &parent.subagent_tool_ceiling_names,
    );
    let allowed_indices = filtered_tools.indices;
    let nested_spawn_ids = filtered_tools.nested_spawn_ids;
    let can_spawn_subagent = filtered_tools.nested_spawn_tools.contains("spawn_subagent");

    // Rule-withheld tools stay callable (`allowed_names`) but off the prompt.
    let child_rules = options
        .run_context
        .for_subagent(definition, config.as_ref().ok().map(AsRef::as_ref))
        .tool_rules;
    let listed = super::super::tool_prep::rule_listed_indices(
        &allowed_indices,
        &parent.all_tools,
        child_rules.as_deref(),
    );
    let mut filtered_specs: Vec<ToolSpec> = listed
        .iter()
        .map(|&i| parent.all_tool_specs[i].as_ref().clone())
        .collect();
    let nested_spawn_tool = if can_spawn_subagent {
        Some(
            crate::agent::orchestration::tools::SpawnSubagentTool::scoped(nested_spawn_ids.clone()),
        )
    } else {
        None
    };
    if let Some(tool) = &nested_spawn_tool {
        if let Some(spec) = filtered_specs
            .iter_mut()
            .find(|spec| spec.name == "spawn_subagent")
        {
            *spec = tool.spec();
        }
    }
    let allowed_names: HashSet<String> = allowed_indices
        .iter()
        .map(|&i| parent.all_tools[i].name().to_string())
        .collect();
    let filtered_specs = dedup_owned_tool_specs(filtered_specs);

    tracing::debug!(
        agent_id = %definition.id,
        model = %model,
        tool_count = allowed_names.len(),
        max_iterations = definition_iter_cap(&options.run_context, definition),
        iteration_policy = ?definition.iteration_policy,
        "[subagent_host:typed] resolved configuration"
    );

    // ── Build the narrow system prompt ─────────────────────────────────
    let render_options = SubagentRenderOptions::from_definition_flags(
        definition.extensions.omit_identity,
        definition.extensions.omit_safety_preamble,
    );

    let connected_integrations_for_prompt: Vec<crate::integrations::ConnectedIntegration> =
        live_integrations
            .iter()
            .filter(|ci| ci.connected)
            .cloned()
            .collect();

    let prompt_tools = super::super::tool_prep::prompt_tools_for(&listed, &parent.all_tools);
    let visible_tool_names: std::collections::HashSet<String> =
        prompt_tools.iter().map(|t| t.name.to_string()).collect();
    let (prompt_tool_call_format, dispatcher_instructions) =
        subagent_prompt_protocol(parent.tool_call_format, &filtered_specs);
    // Load AGENTS.md instruction layers once, at prompt-build time, when the
    // config gate is on. The global layer comes from the workspace dir; the
    // project layer prefers an explicitly inherited workspace root and falls
    // back to the global config `action_dir`. Loading here (not per turn) keeps
    // the sub-agent system
    // prompt byte-stable for prefix caching.
    let agents_md = if parent.agent_config.agents_md_enabled {
        let local_dir = options
            .workspace_descriptor
            .as_ref()
            .or(parent.workspace_descriptor.as_ref())
            .map(|descriptor| descriptor.root.clone())
            .or_else(|| config.as_ref().ok().map(|c| c.action_dir.clone()));
        match local_dir {
            Some(dir) => crate::agent::prompts::load_agents_md_layers(&parent.workspace_dir, &dir),
            None => {
                // No resolvable project dir — still surface the workspace layer.
                tinyagents_harness::context::AgentsMdContent {
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
        tinyagents_harness::context::AgentsMdContent::default()
    };

    let connected_identities_md = options.run_context.connected_identities_md().await;
    let prompt_ctx = PromptContext {
        workspace_dir: &parent.workspace_dir,
        model_name: &model,
        agent_id: &definition.id,
        flow_archetype_text: None,
        tools: &prompt_tools,
        workflows: &parent.workflows,
        dispatcher_instructions: &dispatcher_instructions,
        visible_tool_names: &visible_tool_names,
        tool_call_format: prompt_tool_call_format,
        connected_integrations: &connected_integrations_for_prompt,
        connected_identities_md,
        user_identity: crate::credentials::identity::peek_credential_user_identity(),
        personality_roster: vec![],
        agents_md_global: agents_md.global.clone(),
        agents_md_local: agents_md.local.clone(),
    };

    let archetype_prompt_body = load_typed_prompt_source(definition, config, prompt_ctx).await?;
    let system_prompt = match &definition.system_prompt {
        // The whole prompt, with no sub-agent sections around it.
        PromptSource::Dynamic(_) | PromptSource::Verbatim(_) => archetype_prompt_body,
        PromptSource::Inline(_) | PromptSource::File { .. } => {
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

    let system_prompt = compose_subagent_system_prompt(
        system_prompt,
        &definition.id,
        &visible_tool_names,
        task_prompt,
        options.run_context.spawn_depth,
    );

    // ── Build the user message (with optional context prefix) ──────────
    // Shared one-line stamp (#3602), in the user's zone like the main agent's.
    // On the user message: sub-agent system prompts are byte-stable for caching.
    let zone = config.as_ref().ok().map(|c| c.time_zone());
    let now_str = crate::prompts::current_datetime_line(zone.as_deref());

    let mut context_parts: Vec<&str> = Vec::new();
    if !definition.extensions.omit_memory_context {
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
    let workspace_descriptor = options
        .workspace_descriptor
        .clone()
        .or_else(|| parent.workspace_descriptor.clone());
    if let Some(descriptor) = &workspace_descriptor {
        tracing::debug!(
            agent_id = %definition.id,
            task_id,
            root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[subagent_host] prepared workspace descriptor for tinyagents run"
        );
    }

    let mut child_run_context = options
        .run_context
        .for_subagent(definition, config.as_ref().ok().map(AsRef::as_ref));
    child_run_context.parent = Some(parent.clone());
    let dynamic_tools = nested_spawn_tool
        .map(|tool| vec![Box::new(tool) as Box<dyn Tool>])
        .unwrap_or_default();
    let (output, iterations, agg_usage, early_exit_tool, hit_cap, breaker_halt, failure_cause) =
        super::graph::run_subagent_via_graph(
            subagent_source.clone(),
            &model,
            temperature,
            &mut history,
            parent.all_tools.clone(),
            dynamic_tools,
            filtered_specs.clone(),
            allowed_names,
            definition_iter_cap(&options.run_context, definition),
            (options.run_queue.clone(), options.message_run_queue.clone()),
            parent.on_progress.clone(),
            &definition.id,
            task_id,
            definition.iteration_policy == IterationPolicy::Extended,
            options.thread_id.clone(),
            child_run_context,
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
            // Agent TokenJuice profile → sub-agent context middleware (#4466).
            definition.extensions.tokenjuice_for(&definition.model),
            // The spawn-wide config snapshot supplies the `[context]`
            // knobs the graph used to load for itself.
            config.as_ref().ok().map(|c| c.as_ref()),
            Some(native),
        )
        .await?;

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
        crate::agent::subagent_host::types::SubagentRunStatus::Incomplete {
            reason,
            failure_cause: failure_cause.clone(),
        }
    } else if hit_cap {
        // The tinyagents run stopped at the model-call cap with work still
        // pending (graph summarized a resumable checkpoint into `output`).
        // Surface it as Incomplete so the delegating agent relays the partial
        // result + blocker instead of treating the summary as a finished answer
        // or re-spinning the identical delegation (#4096).
        crate::agent::subagent_host::types::SubagentRunStatus::Incomplete {
            reason: tinyagents_harness::turn_stop::SUBAGENT_ITERATION_CAP_REASON.into(),
            failure_cause: None,
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
    let mut usage = tinyagents_harness::progress::SubagentUsage {
        input_tokens: agg_usage.input_tokens,
        output_tokens: agg_usage.output_tokens,
        cached_input_tokens: agg_usage.cached_input_tokens,
        charged_amount_usd: agg_usage.cost.known_usd,
        cost_source: agg_usage.cost.source,
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
        usage.cost_source = usage.cost_source.max(entry.usage.cost_source);
    }
    Ok(SubagentRunOutcome {
        task_id: task_id.to_string(),
        agent_id: definition.id.clone(),
        output,
        iterations,
        elapsed: started.elapsed(),
        mode: SubagentMode::Typed,
        status,
        failure_cause,
        final_history: history,
        usage,
        // Filled in by the host adapter once the offload step has had its say, so
        // both the harness-offloaded and worker-authored pointers are counted.
        artifact_paths: Vec::new(),
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalExisting,
    })
}
#[cfg(test)]
#[path = "runner_flow_prompt_tests.rs"]
mod flow_prompt_tests;
