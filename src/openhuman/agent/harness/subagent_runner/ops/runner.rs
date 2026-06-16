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

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, IterationPolicy, PromptSource,
};
use crate::openhuman::agent::harness::fork_context::{current_parent, ParentExecutionContext};
use crate::openhuman::agent::harness::subagent_runner::extract_tool::ExtractFromResultTool;
use crate::openhuman::agent::harness::subagent_runner::handoff::ResultHandoffCache;
use crate::openhuman::agent::harness::subagent_runner::tool_prep::{
    filter_tool_indices, is_subagent_spawn_tool, load_prompt_source, top_k_for_toolkit,
};
use crate::openhuman::agent::harness::subagent_runner::types::{
    SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome,
};
use crate::openhuman::agent::harness::{
    current_spawn_depth, with_current_sandbox_mode, with_spawn_depth, MAX_SPAWN_DEPTH,
};
use crate::openhuman::context::prompt::{
    render_subagent_system_prompt, PromptContext, PromptTool, SubagentRenderOptions,
};
use crate::openhuman::file_state::with_file_state_agent_id;
use crate::openhuman::tools::{Tool, ToolCategory, ToolSpec};

use super::loop_::run_inner_loop;
use super::prompt::{append_subagent_role_contract, dedup_tool_specs_by_name};
use super::provider::{
    resolve_subagent_provider, user_is_signed_in_to_composio, LazyToolkitResolver,
};

/// Run a sub-agent based on its definition and a task prompt.
///
/// This is the primary entry point for agent delegation. It performs the following:
/// 1. Resolves the [`ParentExecutionContext`] task-local.
/// 2. Generates a unique `task_id` if one wasn't provided.
/// 3. Dispatches to `run_typed_mode`.
///
/// On success returns a [`SubagentRunOutcome`] whose `output` is the
/// final assistant text. On failure the error is suitable for stringifying
/// into a `tool_result` block.
pub async fn run_subagent(
    definition: &AgentDefinition,
    task_prompt: &str,
    options: SubagentRunOptions,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    // Unconditionally heap-allocate the entire run_subagent body so
    // every caller doesn't have to carry this future's state inline.
    // Tools that delegate run inside the parent agent's already-deep
    // `run_turn_engine` poll, so the parent's stack would otherwise pile
    // (parent engine state + dispatch_subagent state + run_subagent's
    // wrapper state + run_typed_mode state + child engine state) onto
    // tokio's 2 MiB worker stack and abort with "thread
    // 'tokio-rt-worker' has overflowed its stack, fatal runtime error:
    // stack overflow" — observed at `[subagent_runner] dispatching
    // agent_id=researcher ...` in the `chat-harness-subagent` Playwright
    // lane crash. The inner `Box::pin`s around `run_typed_mode` /
    // `run_inner_loop` / child `run_turn_engine` further chunk the
    // child's state so a single sub-agent run can't blow the stack either.
    Box::pin(async move {
        let parent = current_parent().ok_or(SubagentRunError::NoParentContext)?;
        let task_id = options
            .task_id
            .clone()
            .unwrap_or_else(|| format!("sub-{}", uuid::Uuid::new_v4()));
        let started = Instant::now();
        let current_depth = current_spawn_depth();
        let attempted_depth = current_depth.saturating_add(1);

        if attempted_depth > MAX_SPAWN_DEPTH {
            tracing::warn!(
                agent_id = %definition.id,
                task_id = %task_id,
                current_depth,
                attempted_depth,
                max_depth = MAX_SPAWN_DEPTH,
                "[subagent_runner] spawn depth exceeded"
            );
            return Err(SubagentRunError::SpawnDepthExceeded {
                attempted_depth,
                max_depth: MAX_SPAWN_DEPTH,
            });
        }

        tracing::info!(
            agent_id = %definition.id,
            task_id = %task_id,
            spawn_depth = attempted_depth,
            max_spawn_depth = MAX_SPAWN_DEPTH,
            prompt_chars = task_prompt.chars().count(),
            skill_filter = ?options.skill_filter_override.as_deref().or(definition.skill_filter.as_deref()),
            "[subagent_runner] dispatching"
        );

        // Install the sub-agent's declared `sandbox_mode` as the active
        // task-local for every tool invocation inside this run.
        //
        // When the worker opted into git-worktree isolation, also install
        // its isolated checkout path as the `action_dir` override so acting
        // tools (shell, git) operate inside that worktree instead of the
        // shared `Config.action_dir`. When `worktree_action_dir` is `None`
        // (the default / non-isolated path), no override scope is entered and
        // behaviour is unchanged.
        let worktree_action_dir = options.worktree_action_dir.clone();
        if let Some(ref wt_dir) = worktree_action_dir {
            tracing::debug!(
                agent_id = %definition.id,
                task_id = %task_id,
                worktree = %wt_dir.display(),
                "[subagent_runner] installing worktree action_dir override"
            );
        }
        let mut outcome = with_spawn_depth(attempted_depth, async {
            with_file_state_agent_id(task_id.clone(), async {
                with_current_sandbox_mode(definition.sandbox_mode, async {
                    let run = run_typed_mode(definition, task_prompt, &options, &parent, &task_id);
                    match worktree_action_dir {
                        Some(wt_dir) => {
                            crate::openhuman::agent::harness::with_action_dir_override(
                                wt_dir,
                                Box::pin(run),
                            )
                            .await
                        }
                        None => Box::pin(run).await,
                    }
                })
                .await
            })
            .await
        })
        .await?;

        // Truncate result to the definition's cap if set.
        // Use char-count (not byte-length) to avoid panicking on
        // multi-byte UTF-8 sequences at the truncation boundary.
        if let Some(cap) = definition.max_result_chars {
            let original_chars = outcome.output.chars().count();
            if original_chars > cap {
                tracing::debug!(
                    agent_id = %definition.id,
                    original_chars,
                    cap,
                    "[subagent_runner] truncating oversized result to max_result_chars cap"
                );
                let byte_offset = outcome
                    .output
                    .char_indices()
                    .nth(cap)
                    .map(|(i, _)| i)
                    .unwrap_or(outcome.output.len());
                outcome.output.truncate(byte_offset);
                outcome.output.push_str("\n[...truncated]");
            }
        }

        tracing::info!(
            agent_id = %definition.id,
            task_id = %task_id,
            spawn_depth = attempted_depth,
            elapsed_ms = outcome.elapsed.as_millis() as u64,
            iterations = outcome.iterations,
            output_chars = outcome.output.chars().count(),
            "[subagent_runner] completed"
        );

        let _ = started; // silence unused-warning if logging is compiled out
        Ok(outcome)
    })
    .await
}

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
) -> Result<SubagentRunOutcome, SubagentRunError> {
    let started = Instant::now();

    // Resolve provider + model. See `resolve_subagent_provider` for the
    // semantics of each ModelSpec variant. `Config::load_or_init()` is
    // async so the load is hoisted out of the helper — the helper itself
    // is sync and unit-tested.
    let config_loaded = crate::openhuman::config::Config::load_or_init().await;
    let (subagent_provider, model) = resolve_subagent_provider(
        &definition.model,
        &definition.id,
        config_loaded.as_ref().ok(),
        parent.provider.clone(),
        parent.model_name.clone(),
        !definition.subagents.is_empty(),
        options.model_override.as_deref(),
    );
    let temperature = definition.temperature;

    // ── Refresh connected-integrations at spawn time ───────────────────
    //
    // The parent session's `connected_integrations` Vec is frozen at
    // session-start. Re-fetch from the global integrations cache here.
    // The cache is invalidated by `ComposioConnectionCreatedSubscriber`
    // once the OAuth handshake reaches ACTIVE/CONNECTED, so this call
    // returns the fresh list almost for free on the warm path. Fall back
    // to the parent's frozen list when the live fetch returns empty.
    let live_integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration> = {
        let probe_config = crate::openhuman::config::Config::load_or_init().await.ok();
        let signed_in = probe_config
            .as_ref()
            .map(user_is_signed_in_to_composio)
            .unwrap_or(false);
        if !signed_in {
            parent.connected_integrations.clone()
        } else {
            match crate::openhuman::config::Config::load_or_init().await {
                Ok(config) => {
                    use crate::openhuman::composio::FetchConnectedIntegrationsStatus;
                    match crate::openhuman::composio::fetch_connected_integrations_status(&config)
                        .await
                    {
                        FetchConnectedIntegrationsStatus::Authoritative(fresh) => {
                            tracing::debug!(
                                count = fresh.len(),
                                parent_count = parent.connected_integrations.len(),
                                "[subagent_runner] refreshed connected_integrations at spawn time"
                            );
                            fresh
                        }
                        FetchConnectedIntegrationsStatus::Unavailable => {
                            tracing::debug!(
                                "[subagent_runner] integrations backend unavailable; falling back to parent's frozen list"
                            );
                            parent.connected_integrations.clone()
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        "[subagent_runner] config load failed; falling back to parent's frozen integrations list"
                    );
                    parent.connected_integrations.clone()
                }
            }
        }
    };

    // ── Filter tools per definition + per-spawn override ───────────────
    let toolkit_filter = options.toolkit_override.as_deref();
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
            "[subagent_runner] removed sub-agent spawn tools from sub-agent's tool surface"
        );
    }

    // ── Force-include extra_tools ──────────────────────────────────────
    if !definition.extra_tools.is_empty() {
        let disallow_set: std::collections::HashSet<&str> = definition
            .disallowed_tools
            .iter()
            .map(|s| s.as_str())
            .collect();
        for (i, tool) in parent.all_tools.iter().enumerate() {
            let name = tool.name();
            if definition.extra_tools.iter().any(|n| n == name)
                && !allowed_indices.contains(&i)
                && !disallow_set.contains(name)
                && !is_subagent_spawn_tool(name)
            {
                allowed_indices.push(i);
            }
        }
    }

    // ── Dynamic per-action toolkit tools (integrations_agent + toolkit) ──────
    let mut dynamic_tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut lazy_resolver: Option<LazyToolkitResolver> = None;
    let is_integrations_agent_with_toolkit =
        definition.id == "integrations_agent" && toolkit_filter.is_some();

    // `tools_agent` must never see Workflow-category tools.
    if definition.id == "tools_agent" {
        allowed_indices.retain(|&i| parent.all_tools[i].category() != ToolCategory::Workflow);
    }

    if is_integrations_agent_with_toolkit {
        if let Some(tk) = toolkit_filter {
            let arc_config = match crate::openhuman::config::Config::load_or_init().await {
                Ok(c) => std::sync::Arc::new(c),
                Err(e) => {
                    tracing::warn!(
                        agent_id = %definition.id,
                        toolkit = %tk,
                        error = %e,
                        "[subagent_runner:typed] config load failed; dynamic composio tools won't be registered"
                    );
                    return Err(SubagentRunError::Provider(anyhow::anyhow!(
                        "subagent_runner: config load failed building integrations_agent for toolkit `{tk}`: {e}"
                    )));
                }
            };

            use crate::openhuman::composio::client::{create_composio_client, ComposioClientKind};
            let client_kind = match create_composio_client(arc_config.as_ref()) {
                Ok(k) => Some(k),
                Err(e) => {
                    tracing::warn!(
                        agent_id = %definition.id,
                        toolkit = %tk,
                        error = %e,
                        "[subagent_runner:typed] composio factory failed; dynamic per-action tools fall back to cached catalogue"
                    );
                    None
                }
            };

            if let Some(cached_integration) = live_integrations
                .iter()
                .find(|ci| ci.connected && ci.toolkit.eq_ignore_ascii_case(tk))
            {
                let fresh_actions = match &client_kind {
                    Some(ComposioClientKind::Backend(client)) => {
                        match crate::openhuman::composio::fetch_toolkit_actions(client, tk, None)
                            .await
                        {
                            Ok(actions) if !actions.is_empty() => actions,
                            Ok(_) => {
                                tracing::debug!(
                                    agent_id = %definition.id,
                                    toolkit = %tk,
                                    "[subagent_runner:typed] fresh list_tools returned empty; falling back to cached catalogue"
                                );
                                cached_integration.tools.clone()
                            }
                            Err(e) => {
                                tracing::warn!(
                                    agent_id = %definition.id,
                                    toolkit = %tk,
                                    error = %e,
                                    "[subagent_runner:typed] fresh list_tools failed; falling back to cached catalogue"
                                );
                                cached_integration.tools.clone()
                            }
                        }
                    }
                    Some(ComposioClientKind::Direct(_)) => {
                        tracing::info!(
                            agent_id = %definition.id,
                            toolkit = %tk,
                            cached_actions = cached_integration.tools.len(),
                            "[composio-direct] subagent_runner:typed: direct mode active — using cached catalogue, skipping backend list_tools refresh"
                        );
                        cached_integration.tools.clone()
                    }
                    None => {
                        tracing::debug!(
                            agent_id = %definition.id,
                            toolkit = %tk,
                            cached_actions = cached_integration.tools.len(),
                            "[subagent_runner:typed] composio client unavailable; using cached catalogue"
                        );
                        cached_integration.tools.clone()
                    }
                };
                let integration = crate::openhuman::context::prompt::ConnectedIntegration {
                    toolkit: cached_integration.toolkit.clone(),
                    description: cached_integration.description.clone(),
                    tools: fresh_actions,
                    gated_tools: cached_integration.gated_tools.clone(),
                    connected: cached_integration.connected,
                    connections: cached_integration.connections.clone(),
                    non_active_status: cached_integration.non_active_status.clone(),
                };
                let integration = &integration;
                let top_k = top_k_for_toolkit(tk);
                let filter_hits = super::super::super::tool_filter::filter_actions_by_prompt(
                    task_prompt,
                    &integration.tools,
                    top_k,
                );
                let selected: Vec<&crate::openhuman::context::prompt::ConnectedIntegrationTool> =
                    if filter_hits.len() >= super::super::super::tool_filter::MIN_CONFIDENT_HITS {
                        tracing::info!(
                            agent_id = %definition.id,
                            toolkit = %tk,
                            total = integration.tools.len(),
                            kept = filter_hits.len(),
                            top_k = top_k,
                            "[subagent_runner:typed] fuzzy tool filter narrowed toolkit"
                        );
                        filter_hits.iter().map(|&i| &integration.tools[i]).collect()
                    } else {
                        tracing::info!(
                            agent_id = %definition.id,
                            toolkit = %tk,
                            total = integration.tools.len(),
                            filter_hits = filter_hits.len(),
                            "[subagent_runner:typed] fuzzy filter thin; falling back to full toolkit"
                        );
                        integration.tools.iter().collect()
                    };

                for action in selected {
                    dynamic_tools.push(Box::new(
                        crate::openhuman::composio::ComposioActionTool::new(
                            arc_config.clone(),
                            action.name.clone(),
                            action.description.clone(),
                            action.parameters.clone(),
                        ),
                    ));
                }
                tracing::debug!(
                    agent_id = %definition.id,
                    toolkit = %tk,
                    action_count = dynamic_tools.len(),
                    "[subagent_runner:typed] dynamically registered per-action composio tools"
                );
                lazy_resolver = Some(LazyToolkitResolver {
                    config: arc_config.clone(),
                    actions: integration.tools.clone(),
                });
            } else {
                tracing::warn!(
                    agent_id = %definition.id,
                    toolkit = %tk,
                    "[subagent_runner:typed] toolkit not found among parent's connected integrations; sub-agent will have no callable actions (spawn_subagent pre-flight should have caught this)"
                );
            }
        }
    }

    // ── Progressive-disclosure handoff cache ───────────────────────────
    let handoff_cache: Option<Arc<ResultHandoffCache>> = if is_integrations_agent_with_toolkit {
        let cache = Arc::new(ResultHandoffCache::new());
        let parent_chain = match parent.session_parent_prefix.as_deref() {
            Some(prefix) => format!("{}__{}", prefix, parent.session_key),
            None => parent.session_key.clone(),
        };
        dynamic_tools.push(Box::new(ExtractFromResultTool::new(
            cache.clone(),
            parent.provider.clone(),
            parent.workspace_dir.clone(),
            parent_chain,
            definition.id.clone(),
        )));
        tracing::debug!(
            agent_id = %definition.id,
            "[subagent_runner:typed] registered extract_from_result tool + handoff cache"
        );
        Some(cache)
    } else {
        None
    };

    // Build provider-visible tool schemas in EXECUTION-PRECEDENCE order:
    // `dynamic_tools` (extra_tools at runtime) before parent specs.
    let mut filtered_specs: Vec<ToolSpec> = dynamic_tools.iter().map(|t| t.spec()).collect();
    filtered_specs.extend(
        allowed_indices
            .iter()
            .map(|&i| parent.all_tool_specs[i].clone()),
    );
    let mut allowed_names: HashSet<String> = allowed_indices
        .iter()
        .map(|&i| parent.all_tools[i].name().to_string())
        .collect();
    // Dynamic tool names must also be in the allowlist so the inner loop
    // accepts model tool_calls that reference them.
    for tool in &dynamic_tools {
        allowed_names.insert(tool.name().to_string());
    }
    let filtered_specs =
        crate::openhuman::agent::harness::session::dedup_visible_tool_specs(filtered_specs);
    let filtered_specs = dedup_tool_specs_by_name(&definition.id, filtered_specs);

    tracing::debug!(
        agent_id = %definition.id,
        model = %model,
        tool_count = allowed_names.len(),
        max_iterations = definition.effective_max_iterations(),
        iteration_policy = ?definition.iteration_policy,
        "[subagent_runner:typed] resolved configuration"
    );

    // ── Build the narrow system prompt ─────────────────────────────────
    let render_options = SubagentRenderOptions::from_definition_flags(
        definition.omit_identity,
        definition.omit_safety_preamble,
        definition.omit_skills_catalog,
        definition.omit_profile,
        definition.omit_memory_md,
    );

    let narrowed_integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration> =
        match toolkit_filter {
            Some(tk) => live_integrations
                .iter()
                .filter(|ci| ci.connected && ci.toolkit.eq_ignore_ascii_case(tk))
                .cloned()
                .collect(),
            None => live_integrations
                .iter()
                .filter(|ci| ci.connected)
                .cloned()
                .collect(),
        };

    let prompt_tools: Vec<PromptTool<'_>> = allowed_indices
        .iter()
        .map(|&i| {
            let t = parent.all_tools[i].as_ref();
            PromptTool {
                name: t.name(),
                description: t.description(),
                parameters_schema: Some(t.parameters_schema().to_string()),
            }
        })
        .chain(dynamic_tools.iter().map(|t| PromptTool {
            name: t.name(),
            description: t.description(),
            parameters_schema: Some(t.parameters_schema().to_string()),
        }))
        .collect();
    let visible_tool_names: std::collections::HashSet<String> =
        prompt_tools.iter().map(|t| t.name.to_string()).collect();
    let dispatcher_instructions = {
        use crate::openhuman::agent::dispatcher::{
            NativeToolDispatcher, PFormatToolDispatcher, ToolDispatcher, XmlToolDispatcher,
        };
        use crate::openhuman::agent::pformat::PFormatRegistry;
        use crate::openhuman::context::prompt::ToolCallFormat;
        let empty_tools: Vec<Box<dyn Tool>> = Vec::new();
        match parent.tool_call_format {
            ToolCallFormat::PFormat => {
                PFormatToolDispatcher::new(PFormatRegistry::new()).prompt_instructions(&empty_tools)
            }
            ToolCallFormat::Native => NativeToolDispatcher.prompt_instructions(&empty_tools),
            ToolCallFormat::Json => XmlToolDispatcher.prompt_instructions(&empty_tools),
        }
    };
    let prompt_ctx = PromptContext {
        workspace_dir: &parent.workspace_dir,
        model_name: &model,
        agent_id: &definition.id,
        tools: &prompt_tools,
        workflows: &parent.workflows,
        dispatcher_instructions: &dispatcher_instructions,
        learned: crate::openhuman::context::prompt::LearnedContextData::default(),
        visible_tool_names: &visible_tool_names,
        tool_call_format: parent.tool_call_format,
        connected_integrations: &narrowed_integrations,
        connected_identities_md: crate::openhuman::agent::prompts::render_connected_identities(),
        include_profile: !definition.omit_profile,
        include_memory_md: !definition.omit_memory_md,
        curated_snapshot: None,
        user_identity: crate::openhuman::app_state::peek_cached_current_user_identity(),
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
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
            render_subagent_system_prompt(
                &parent.workspace_dir,
                &model,
                &allowed_indices,
                &parent.all_tools,
                &dynamic_tools,
                &archetype_prompt_body,
                render_options,
                parent.tool_call_format,
                &narrowed_integrations,
            )
        }
    };

    let system_prompt = append_subagent_role_contract(system_prompt, &definition.id);

    // ── Build the user message (with optional context prefix) ──────────
    let now = chrono::Local::now();
    let now_str = format!(
        "Current Date & Time: {} ({})",
        now.format("%Y-%m-%d %H:%M:%S"),
        now.format("%Z")
    );

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
    let mut history: Vec<crate::openhuman::inference::provider::ChatMessage> =
        if let Some(ref initial) = options.initial_history {
            tracing::info!(
                agent_id = %definition.id,
                task_id = %task_id,
                history_len = initial.len(),
                "[subagent_runner] resuming with initial_history (checkpoint replay)"
            );
            initial.clone()
        } else {
            let user_message = if context_parts.is_empty() {
                task_prompt.to_string()
            } else {
                format!("[Context]\n{}\n\n{task_prompt}", context_parts.join("\n\n"))
            };
            vec![
                crate::openhuman::inference::provider::ChatMessage::system(system_prompt),
                crate::openhuman::inference::provider::ChatMessage::user(user_message),
            ]
        };

    // ── Run the inner tool-call loop ───────────────────────────────────
    // Resolve the sub-agent model's user-configured vision flag; defaults to
    // `false` when config can't be loaded. Combined with the provider capability
    // at the gate, this lets a flagged custom/BYOK sub-agent model forward images.
    let model_vision = crate::openhuman::config::Config::load_or_init()
        .await
        .ok()
        .map(|cfg| crate::openhuman::inference::model_context::model_supports_vision(&model, &cfg))
        .unwrap_or(false);
    tracing::debug!(
        target: "subagent_runner",
        model = %model,
        model_vision,
        "[subagent_runner] resolved sub-agent model vision capability"
    );
    let (output, iterations, _agg_usage, early_exit_tool) = Box::pin(run_inner_loop(
        subagent_provider.as_ref(),
        &mut history,
        &parent.all_tools,
        dynamic_tools,
        &filtered_specs,
        allowed_names,
        lazy_resolver,
        &model,
        model_vision,
        temperature,
        definition.effective_max_iterations(),
        task_id,
        &definition.id,
        options.worker_thread_id.clone(),
        handoff_cache.as_deref(),
        parent,
        definition.iteration_policy == IterationPolicy::Extended,
        options.run_queue.clone(),
    ))
    .await?;

    // Determine status: if the turn engine exited early because of
    // ask_user_clarification, checkpoint the history and return
    // AwaitingUser so the orchestrator can relay the user's answer.
    let status = if early_exit_tool.as_deref() == Some("ask_user_clarification") {
        let question = output.clone();
        let options_vec: Option<Vec<String>> = None;

        let checkpoint_dir = options
            .checkpoint_dir
            .clone()
            .unwrap_or_else(|| parent.workspace_dir.join(".openhuman/subagent_checkpoints"));
        if let Err(e) = std::fs::create_dir_all(&checkpoint_dir) {
            tracing::warn!(
                task_id = %task_id,
                error = %e,
                "[subagent_runner] failed to create checkpoint directory"
            );
        } else {
            let checkpoint_data =
                crate::openhuman::agent::harness::subagent_runner::types::SubagentCheckpointData {
                    task_id: task_id.to_string(),
                    agent_id: definition.id.clone(),
                    worker_thread_id: options.worker_thread_id.clone(),
                    history: history.clone(),
                    question: question.clone(),
                    options: options_vec.clone(),
                    toolkit_override: options.toolkit_override.clone(),
                    skill_filter_override: options.skill_filter_override.clone(),
                    model_override: options.model_override.clone(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
            let checkpoint_path = checkpoint_dir.join(format!("{task_id}.json"));
            match serde_json::to_string_pretty(&checkpoint_data) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&checkpoint_path, json) {
                        tracing::warn!(
                            task_id = %task_id,
                            path = %checkpoint_path.display(),
                            error = %e,
                            "[subagent_runner] failed to write checkpoint"
                        );
                    } else {
                        tracing::info!(
                            task_id = %task_id,
                            path = %checkpoint_path.display(),
                            history_len = history.len(),
                            "[subagent_runner] checkpoint written for awaiting_user"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %task_id,
                        error = %e,
                        "[subagent_runner] failed to serialize checkpoint"
                    );
                }
            }
        }

        crate::openhuman::agent::harness::subagent_runner::types::SubagentRunStatus::AwaitingUser {
            question,
            options: options_vec,
        }
    } else {
        crate::openhuman::agent::harness::subagent_runner::types::SubagentRunStatus::Completed
    };

    Ok(SubagentRunOutcome {
        task_id: task_id.to_string(),
        agent_id: definition.id.clone(),
        output,
        iterations,
        elapsed: started.elapsed(),
        mode: SubagentMode::Typed,
        status,
    })
}
