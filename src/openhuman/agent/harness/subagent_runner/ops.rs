//! Sub-agent execution entry points and the inner tool-call loop.
//!
//! The public runner lives in [`run_subagent`]. It dispatches to
//! [`run_typed_mode`] (narrow prompt + filtered tools) or
//! [`run_fork_mode`] (prefix-replay) depending on the
//! [`super::types::SubagentMode`] implied by the
//! [`crate::openhuman::agent::harness::definition::AgentDefinition`].
//!
//! Both modes delegate to [`run_inner_loop`] which drives provider
//! calls and tool execution until the model returns without further
//! tool calls (or the iteration budget is exhausted).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use super::super::fork_context::{
    current_fork, current_parent, ForkContext, ParentExecutionContext,
};
use super::super::session::transcript;
use super::extract_tool::ExtractFromResultTool;
use super::handoff::{
    build_handoff_placeholder, clean_tool_output, ResultHandoffCache,
    HANDOFF_OVERSIZE_THRESHOLD_TOKENS,
};
use super::tool_prep::{
    build_text_mode_tool_instructions, filter_tool_indices, is_subagent_spawn_tool,
    is_welcome_only_tool, load_prompt_source, top_k_for_toolkit,
};
use super::types::{SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome};
use crate::openhuman::agent::harness::definition::{AgentDefinition, PromptSource};
use crate::openhuman::agent::harness::with_current_sandbox_mode;
use crate::openhuman::context::prompt::{
    render_subagent_system_prompt, PromptContext, PromptTool, SubagentRenderOptions,
};
use crate::openhuman::providers::{ChatMessage, ChatRequest, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolCategory, ToolSpec};

/// Run a sub-agent based on its definition and a task prompt.
///
/// This is the primary entry point for agent delegation. It performs the following:
/// 1. Resolves the [`ParentExecutionContext`] task-local.
/// 2. Generates a unique `task_id` if one wasn't provided.
/// 3. Dispatches to either `run_fork_mode` or `run_typed_mode` based on the definition.
///
/// On success returns a [`SubagentRunOutcome`] whose `output` is the
/// final assistant text. On failure the error is suitable for stringifying
/// into a `tool_result` block.
pub async fn run_subagent(
    definition: &AgentDefinition,
    task_prompt: &str,
    options: SubagentRunOptions,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    let parent = current_parent().ok_or(SubagentRunError::NoParentContext)?;
    let task_id = options
        .task_id
        .clone()
        .unwrap_or_else(|| format!("sub-{}", uuid::Uuid::new_v4()));
    let started = Instant::now();

    tracing::info!(
        agent_id = %definition.id,
        task_id = %task_id,
        prompt_chars = task_prompt.chars().count(),
        skill_filter = ?options.skill_filter_override.as_deref().or(definition.skill_filter.as_deref()),
        "[subagent_runner] dispatching"
    );

    // Install the sub-agent's declared `sandbox_mode` as the active
    // task-local for every tool invocation inside this run. Tools that
    // want to gate on it (e.g. `composio_execute` rejecting
    // Write/Admin slugs under `ReadOnly`) read it via
    // `current_sandbox_mode()`; tools that don't care just ignore it.
    let outcome = with_current_sandbox_mode(definition.sandbox_mode, async {
        if definition.uses_fork_context {
            let fork = current_fork().ok_or(SubagentRunError::NoForkContext)?;
            run_fork_mode(definition, task_prompt, &options, &parent, &fork, &task_id).await
        } else {
            run_typed_mode(definition, task_prompt, &options, &parent, &task_id).await
        }
    })
    .await?;

    tracing::info!(
        agent_id = %definition.id,
        task_id = %task_id,
        elapsed_ms = outcome.elapsed.as_millis() as u64,
        iterations = outcome.iterations,
        output_chars = outcome.output.chars().count(),
        "[subagent_runner] completed"
    );

    let _ = started; // silence unused-warning if logging is compiled out
    Ok(outcome)
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

    // ── Resolve model + temperature ────────────────────────────────────
    let model = definition.model.resolve(&parent.model_name);
    let temperature = definition.temperature;

    // Archetype prompt loading is deferred until AFTER tool filtering so
    // dynamic builders receive the final, filtered tool list (rather
    // than the parent's full registry). The actual
    // `load_prompt_source(...)` call lives just above
    // `render_subagent_system_prompt` below.

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

    // `complete_onboarding` is a welcome-only tool — it flips the
    // onboarding-complete flag in workspace config and is meaningless
    // (and potentially destructive) from any other agent. Strip it
    // from every non-welcome subagent regardless of their scope.
    if definition.id != "welcome" {
        allowed_indices.retain(|&i| !is_welcome_only_tool(parent.all_tools[i].name()));
    }

    // Sub-agents must never spawn their own sub-agents. Nested spawns
    // create a recursion tree the harness doesn't budget, observe, or
    // cost-attribute — and historically produced runaway dispatch loops
    // (e.g. summarizer → summarizer → …). The orchestrator is the only
    // node that delegates; every archetype running here is, by
    // definition, a sub-agent. Strip `spawn_subagent` and every
    // synthesised `delegate_*` tool regardless of the archetype's
    // declared scope. This is belt-and-braces: archetype definitions
    // should not list these tools either, but we enforce it here so a
    // misconfigured TOML can't bypass the rule.
    let before = allowed_indices.len();
    allowed_indices.retain(|&i| !is_subagent_spawn_tool(parent.all_tools[i].name()));
    let stripped = before - allowed_indices.len();
    if stripped > 0 {
        tracing::debug!(
            agent_id = %definition.id,
            stripped,
            "[subagent_runner] removed sub-agent spawn tools from sub-agent's tool surface"
        );
    }

    // ── Force-include extra_tools ──────────────────────────────────────
    //
    // `extra_tools` is a simple "also include these" hook that bypasses
    // [`ToolScope`] / [`AgentDefinition::skill_filter`] but still honours
    // `disallowed_tools`. Historically this was the bypass list for the
    // now-removed `category_filter`; it remains useful for custom
    // definitions that want to add a couple of named tools on top of a
    // narrow scope.
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
                // `extra_tools` cannot be used to bypass the sub-agent
                // spawn guard above — a stray TOML entry listing
                // `spawn_subagent` there must still be dropped.
                && !is_subagent_spawn_tool(name)
            {
                allowed_indices.push(i);
            }
        }
    }

    // ── Dynamic per-action toolkit tools (integrations_agent + toolkit) ──────
    //
    // When `integrations_agent` is spawned with a `toolkit` argument (e.g.
    // `toolkit="gmail"`), build one [`ComposioActionTool`] per action
    // in that toolkit and inject them into the sub-agent's tool list.
    // Each carries the action's real JSON schema, so the LLM's native
    // tool-calling path validates arguments before they hit the wire
    // — no more "guess parameters from prose then dispatch through
    // composio_execute" round-trips.
    //
    // Generic dispatchers (`composio_execute`, `composio_list_tools`)
    // are stripped from the parent-filtered indices in this path so
    // the model only sees one way to call each action.
    let mut dynamic_tools: Vec<Box<dyn Tool>> = Vec::new();
    let is_integrations_agent_with_toolkit =
        definition.id == "integrations_agent" && toolkit_filter.is_some();

    // `tools_agent` is the Composio-free counterpart to
    // `integrations_agent`: it inherits the orchestrator's wildcard
    // scope but must never see Skill-category tools. Stripping them
    // here (before any dynamic additions) keeps the parent-fed
    // `allowed_indices` clean of composio_* meta-tools and
    // toolkit-specific action tools. Delegation to integrations_agent
    // is the orchestrator's job, not this agent's.
    if definition.id == "tools_agent" {
        allowed_indices.retain(|&i| parent.all_tools[i].category() != ToolCategory::Skill);
    }

    if is_integrations_agent_with_toolkit {
        // Tool visibility is fully governed by the TOML scope
        // (`agent.tools.named = [...]` on the integrations_agent
        // definition) plus the dynamic per-action ComposioActionTools
        // injected below. Anything the agent author explicitly named
        // in the TOML is kept as-is — no extra stripping here.
        // Previously we dropped every Skill-category tool at this
        // point, which also dropped `composio_list_tools` /
        // `composio_execute` whenever they were declared in the TOML,
        // making the TOML changes look like no-ops.

        if let (Some(tk), Some(client)) = (toolkit_filter, parent.composio_client.as_ref()) {
            // The spawn_subagent pre-flight already verified the
            // toolkit is in the allowlist AND has an active
            // connection, so the matching entry must be present and
            // marked connected. Defensive lookup anyway.
            if let Some(cached_integration) = parent
                .connected_integrations
                .iter()
                .find(|ci| ci.connected && ci.toolkit.eq_ignore_ascii_case(tk))
            {
                // Refresh the toolkit's action catalogue at spawn time
                // by calling `composio_list_tools` for the bound toolkit.
                // The cached list on `parent.connected_integrations`
                // comes from the session-start bulk fetch, which can
                // return zero actions for some toolkits even when the
                // per-toolkit endpoint returns a full catalogue. Falling
                // back to the cached list preserves the previous
                // behaviour on network failure.
                let fresh_actions = match crate::openhuman::composio::fetch_toolkit_actions(
                    client, tk,
                )
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
                };
                let integration = crate::openhuman::context::prompt::ConnectedIntegration {
                    toolkit: cached_integration.toolkit.clone(),
                    description: cached_integration.description.clone(),
                    tools: fresh_actions,
                    connected: cached_integration.connected,
                };
                let integration = &integration;
                // Fuzzy-filter the toolkit's actions against the task prompt
                // so large catalogues (e.g. github ~500 actions) are narrowed
                // to the handful actually relevant to this delegation. The
                // orchestrator's `SkillDelegationTool` schema forces the
                // prompt to be a clear, context-rich instruction, so it's a
                // reliable matching target.
                //
                // Heavy-schema toolkits (Gmail, Notion, GitHub, Salesforce,
                // HubSpot, Google Workspace, Microsoft Teams) ship per-action
                // JSON schemas so dense that even a moderate top-K blows the
                // request past Fireworks' 65 535-rule grammar cap in native
                // mode and the 196 607-token context cap in text mode. Tight
                // top-K of 12 keeps those toolkits inside both ceilings while
                // still giving the fuzzy scorer room for adjacent matches.
                // Lighter toolkits (reddit, slack, linear, telegram, …) keep
                // the looser top-K of 25.
                //
                // Fallback: if the filter yields fewer than
                // `MIN_CONFIDENT_HITS` results, register every action. A
                // too-narrow filter is worse than none — it starves the
                // sub-agent and forces it to guess.
                let top_k = top_k_for_toolkit(tk);
                let filter_hits = super::super::tool_filter::filter_actions_by_prompt(
                    task_prompt,
                    &integration.tools,
                    top_k,
                );
                let selected: Vec<&crate::openhuman::context::prompt::ConnectedIntegrationTool> =
                    if filter_hits.len() >= super::super::tool_filter::MIN_CONFIDENT_HITS {
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
                            client.clone(),
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
            } else {
                tracing::warn!(
                    agent_id = %definition.id,
                    toolkit = %tk,
                    "[subagent_runner:typed] toolkit not found among parent's connected integrations; sub-agent will have no callable actions (spawn_subagent pre-flight should have caught this)"
                );
            }
        } else if toolkit_filter.is_some() {
            tracing::warn!(
                agent_id = %definition.id,
                "[subagent_runner:typed] toolkit requested but composio client is unavailable on parent context"
            );
        }
    }

    // ── Progressive-disclosure handoff cache ───────────────────────────
    //
    // Built only for integrations_agent-with-toolkit because that's the only
    // typed sub-agent that regularly calls external tools capable of
    // returning megabyte-scale payloads (Composio actions). Every other
    // typed sub-agent gets `None` and its tool results stay inline.
    //
    // When enabled, oversized tool results get stashed into this cache
    // and their place in history is taken by a short placeholder (see
    // `build_handoff_placeholder`). The sub-agent can then call the
    // companion `extract_from_result` tool below to run a direct
    // provider call against the cached payload with a targeted query.
    // Lazy / pay-per-question, so trivial asks answerable from the
    // preview don't pay any extra LLM cost.
    let handoff_cache: Option<Arc<ResultHandoffCache>> = if is_integrations_agent_with_toolkit {
        let cache = Arc::new(ResultHandoffCache::new());

        // `extract_from_result` is now a pure tool — it takes the
        // parent's provider and calls `chat_with_system` directly
        // against the extraction model, instead of spawning the
        // `summarizer` sub-agent. Removes an entire layer of harness
        // scaffolding (system prompt assembly, tool-loop, recursion
        // guards) that this workload never needed.
        //
        // Transcript plumbing: the extraction LLM still costs tokens,
        // so each call writes a self-contained transcript under
        // `session_raw/DDMMYYYY/` (and its companion `.md`) keyed by
        // the parent chain, to match the rest of the session tree.
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

    let mut filtered_specs: Vec<ToolSpec> = allowed_indices
        .iter()
        .map(|&i| parent.all_tool_specs[i].clone())
        .collect();
    let mut allowed_names: HashSet<String> = allowed_indices
        .iter()
        .map(|&i| parent.all_tools[i].name().to_string())
        .collect();
    // Append dynamic tool specs / names so they're discoverable by the
    // provider (native tool-calling) and by the inner loop's allowlist.
    for tool in &dynamic_tools {
        filtered_specs.push(tool.spec());
        allowed_names.insert(tool.name().to_string());
    }

    tracing::debug!(
        agent_id = %definition.id,
        model = %model,
        tool_count = allowed_names.len(),
        max_iterations = definition.max_iterations,
        "[subagent_runner:typed] resolved configuration"
    );

    // ── Build the narrow system prompt ─────────────────────────────────
    //
    // The renderer lives in `context::prompt` alongside the rest of
    // the system-prompt code so all prompt assembly has one home.
    // We still use the purpose-built narrow renderer rather than the
    // general `SystemPromptBuilder::for_subagent` because the builder
    // requires a slice of `Box<dyn Tool>` and we only have indices
    // into the parent's vec (Box isn't Clone, so we can't build an
    // owning filtered slice cheaply).
    //
    // Per-definition omit_* flags are threaded through via
    // `SubagentRenderOptions` — previously the narrow renderer
    // hard-coded all three as "omit", which silently downgraded
    // definitions like `code_executor` / `tool_maker` / `integrations_agent`
    // that set `omit_safety_preamble = false`.
    let render_options = SubagentRenderOptions::from_definition_flags(
        definition.omit_identity,
        definition.omit_safety_preamble,
        definition.omit_skills_catalog,
        definition.omit_profile,
        definition.omit_memory_md,
    );

    // Sub-agent prompt rendering: only ever surface CONNECTED
    // integrations. When narrowed to a specific toolkit, we further
    // restrict to that one entry. Not-connected entries belong only
    // in the orchestrator's Delegation Guide; they have no place in
    // a sub-agent that's actually executing work.
    let narrowed_integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration> =
        match toolkit_filter {
            Some(tk) => parent
                .connected_integrations
                .iter()
                .filter(|ci| ci.connected && ci.toolkit.eq_ignore_ascii_case(tk))
                .cloned()
                .collect(),
            None => parent
                .connected_integrations
                .iter()
                .filter(|ci| ci.connected)
                .cloned()
                .collect(),
        };
    // ── Resolve archetype prompt body (post-filter) ────────────────────
    //
    // Build a live [`PromptContext`] — same shape the main agent uses
    // on every turn — so `Dynamic` builders can compose the full
    // system prompt via the section helpers in
    // [`crate::openhuman::context::prompt`]. `Inline` / `File` sources
    // continue to use the legacy `render_subagent_system_prompt`
    // wrapper.
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
    // Derive the visible-tool set from the prompt tool list so prompt
    // sections that gate on `visible_tool_names` (e.g. tool-protocol
    // notes) see exactly what the model sees, rather than an empty set.
    let visible_tool_names: std::collections::HashSet<String> =
        prompt_tools.iter().map(|t| t.name.to_string()).collect();
    // Match the main-agent turn (`session/turn.rs::build_system_prompt`)
    // by supplying the dispatcher's protocol instructions here. Dynamic
    // prompt builders route tools through `render_tools(ctx)`, which
    // appends `ctx.dispatcher_instructions` after the tool catalogue —
    // passing an empty string drops the `## Tool Use Protocol` block and
    // leaves PFormat/Json sub-agents with no call-format guidance.
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
        skills: &parent.skills,
        dispatcher_instructions: &dispatcher_instructions,
        learned: crate::openhuman::context::prompt::LearnedContextData::default(),
        visible_tool_names: &visible_tool_names,
        tool_call_format: parent.tool_call_format,
        connected_integrations: &narrowed_integrations,
        connected_identities_md: crate::openhuman::agent::prompts::render_connected_identities(),
        include_profile: !definition.omit_profile,
        include_memory_md: !definition.omit_memory_md,
    };

    let system_prompt = match &definition.system_prompt {
        PromptSource::Dynamic(build) => {
            // Function-driven builder returns the final prompt text.
            build(&prompt_ctx).map_err(|e| SubagentRunError::PromptLoad {
                path: format!("<dynamic:{}>", definition.id),
                source: std::io::Error::other(e.to_string()),
            })?
        }
        PromptSource::Inline(_) | PromptSource::File { .. } => {
            // Legacy path for TOML-authored agents: load the raw body,
            // then wrap it with the canonical section layout.
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

    // ── Build the user message (with optional context prefix) ──────────
    // Merge explicit orchestrator context with the parent's auto-loaded
    // memory context, but only when the definition opts into memory
    // inheritance.
    let now = chrono::Local::now();
    let now_str = format!(
        "Current Date & Time: {} ({})",
        now.format("%Y-%m-%d %H:%M:%S"),
        now.format("%Z")
    );

    let mut context_parts: Vec<&str> = Vec::new();
    if !definition.omit_memory_context {
        if let Some(ref mem_ctx) = parent.memory_context {
            context_parts.push(mem_ctx);
        }
    }

    // Always include temporal context for typed sub-agents. System prompts
    // for sub-agents are byte-stable for KV cache reuse, so "now" must
    // ride in the user message.
    context_parts.push(&now_str);

    if let Some(ref ctx) = options.context {
        context_parts.push(ctx);
    }
    let user_message = if context_parts.is_empty() {
        task_prompt.to_string()
    } else {
        format!("[Context]\n{}\n\n{task_prompt}", context_parts.join("\n\n"))
    };

    let mut history: Vec<ChatMessage> = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(user_message),
    ];

    // ── Run the inner tool-call loop ───────────────────────────────────
    // Transcript persistence lives INSIDE the loop (one write per
    // provider response), mirroring the main-agent turn loop in
    // `session/turn.rs`. No post-loop write needed here.
    let (output, iterations, _agg_usage) = run_inner_loop(
        parent.provider.as_ref(),
        &mut history,
        &parent.all_tools,
        &dynamic_tools,
        &filtered_specs,
        &allowed_names,
        &model,
        temperature,
        definition.max_iterations,
        task_id,
        &definition.id,
        handoff_cache.as_deref(),
        parent,
    )
    .await?;

    Ok(SubagentRunOutcome {
        task_id: task_id.to_string(),
        agent_id: definition.id.clone(),
        output,
        iterations,
        elapsed: started.elapsed(),
        mode: SubagentMode::Typed,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Fork mode — replay parent's bytes for prefix-cache reuse
// ─────────────────────────────────────────────────────────────────────────────

/// Execute a sub-agent in "Fork" mode.
///
/// This mode is an optimization. It replays the parent's EXACT rendered prompt
/// and history prefix up to the point of delegation. This allows the inference
/// server to reuse its existing KV-cache for the prefix, drastically reducing
/// first-token latency and token costs for parallel delegation.
async fn run_fork_mode(
    definition: &AgentDefinition,
    _task_prompt: &str,
    _options: &SubagentRunOptions,
    parent: &ParentExecutionContext,
    fork: &ForkContext,
    task_id: &str,
) -> Result<SubagentRunOutcome, SubagentRunError> {
    let started = Instant::now();

    // The fork's task prompt comes from the ForkContext (set by the
    // parent's tool-dispatch site), not from the spawn_subagent args
    // directly. This guarantees the bytes the parent committed to are
    // what the child sees.
    let fork_task_prompt = fork.fork_task_prompt.clone();

    tracing::debug!(
        agent_id = %definition.id,
        prefix_len = fork.message_prefix.len(),
        "[subagent_runner:fork] replaying parent prefix"
    );

    // History = parent's exact prefix (which already starts with the
    // parent's system message), then the new fork directive as a user
    // message. The system_prompt arc is unused here because the prefix
    // already contains the system message at index 0 — but we sanity-
    // check that invariant.
    debug_assert!(
        fork.message_prefix
            .first()
            .map(|m| m.role == "system")
            .unwrap_or(false),
        "fork message_prefix must start with the parent's system message"
    );
    let mut history: Vec<ChatMessage> = (*fork.message_prefix).clone();
    history.push(ChatMessage::user(fork_task_prompt));

    // Fork mode keeps the parent's exact tool schema snapshot so the
    // request body matches the prefix the backend has already cached.
    // Runtime execution still resolves against the parent's live tool
    // registry.
    //
    // Sub-agents (including fork-mode ones) must not spawn their own
    // sub-agents — the rule that applies in `run_typed_mode`'s filter
    // applies here too. We keep `spawn_subagent` / `delegate_*` in
    // `fork.tool_specs` so the prefix bytes still match the parent's
    // cached body (mutating the specs would defeat the whole point of
    // fork mode), and instead drop them from `allowed_names` so the
    // runtime rejects any attempt to call them with the usual
    // "not in allowlist" path.
    let allowed_names: HashSet<String> = parent
        .all_tools
        .iter()
        .map(|t| t.name().to_string())
        .filter(|name| !is_subagent_spawn_tool(name))
        .collect();

    let model = parent.model_name.clone();
    let temperature = parent.temperature;
    // Use the parent's iteration cap, not the synthetic fork definition's.
    let max_iterations = parent.agent_config.max_tool_iterations.max(1);

    // Fork mode replays the parent's exact tool list — no dynamic
    // toolkit-scoped tools, so `extra_tools` is empty.
    let fork_extra_tools: Vec<Box<dyn Tool>> = Vec::new();
    // Transcript persistence happens per-iteration inside
    // `run_inner_loop`; no post-loop write needed.
    let (output, iterations, _agg_usage) = run_inner_loop(
        parent.provider.as_ref(),
        &mut history,
        &parent.all_tools,
        &fork_extra_tools,
        fork.tool_specs.as_slice(),
        &allowed_names,
        &model,
        temperature,
        max_iterations,
        task_id,
        &definition.id,
        None,
        parent,
    )
    .await?;

    Ok(SubagentRunOutcome {
        task_id: task_id.to_string(),
        agent_id: definition.id.clone(),
        output,
        iterations,
        elapsed: started.elapsed(),
        mode: SubagentMode::Fork,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Inner tool-call loop (slim version of agent::loop_::tool_loop)
// ─────────────────────────────────────────────────────────────────────────────

/// Cumulative usage stats gathered across all provider calls in the loop.
#[derive(Debug, Clone, Default)]
struct AggregatedUsage {
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    charged_amount_usd: f64,
}

/// The sub-agent's private tool-execution engine.
///
/// This function drives the iterative cycle of:
/// 1. Sending messages to the provider.
/// 2. Parsing the provider's response for tool calls.
/// 3. Executing tools (with sandboxing and timeouts).
/// 4. Appending results to history and looping until a final response is found.
///
/// Unlike the main agent loop, this is isolated and returns only the final text
/// to be synthesized by the parent.
#[allow(clippy::too_many_arguments)]
async fn run_inner_loop(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    parent_tools: &[Box<dyn Tool>],
    extra_tools: &[Box<dyn Tool>],
    tool_specs: &[ToolSpec],
    allowed_names: &HashSet<String>,
    model: &str,
    temperature: f64,
    max_iterations: usize,
    task_id: &str,
    agent_id: &str,
    handoff_cache: Option<&ResultHandoffCache>,
    parent: &ParentExecutionContext,
) -> Result<(String, usize, AggregatedUsage), SubagentRunError> {
    let max_iterations = max_iterations.max(1);

    // Sub-agent transcript stem — mirrors what
    // `persist_subagent_transcript` used to compute on one-shot
    // post-loop writes. We compute it once up front so **every
    // iteration's** persist call resolves to the same file on disk:
    //   `{parent_chain}__{unix_ts}_{agent_id}.jsonl`.
    let child_session_key = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let unix_ts = now.as_secs();
        // Nanos component + task_id suffix disambiguate sibling sub-agents
        // spawned within the same wall-clock second (tests and fan-out
        // flows routinely do this, and a shared stem would overwrite the
        // earlier sibling's transcript file).
        let nanos = now.subsec_nanos();
        let sanitized: String = agent_id
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

    // ── Text-mode override for integrations_agent ────────────────────────────
    //
    // Large Composio toolkits (Notion, Salesforce, HubSpot, GitHub) ship
    // per-action JSON schemas that are extraordinarily dense — deeply
    // nested object/block types, recursive refs, huge discriminated
    // unions. Fireworks-style providers (which the backend forwards to)
    // auto-compile every entry in `tools: [...]` into a grammar and
    // index rules with a `uint16_t` — max 65 535 rules. Even with the
    // upstream fuzzy filter narrowing Notion 48 → 16, a single request
    // generates 100 000+ rules and the provider rejects it with 400
    // before generation starts.
    //
    // The fuzzy filter can't fix this because the bound is per-action,
    // not per-toolkit: one Notion schema alone can produce thousands of
    // rules. The only client-side lever is to **not send `tools: [...]`
    // at all** — the backend has nothing to compile, so no grammar, so
    // no ceiling. We then describe the tools in the system prompt as
    // prose (XmlToolDispatcher format) and parse `<tool_call>` tags out
    // of the model's free-form response text.
    //
    // Scoped to `integrations_agent` because that's the only path where we
    // pass Composio toolkit schemas. Every other typed sub-agent
    // (welcome, researcher, summarizer, …) uses small built-in tool
    // sets that stay well under the grammar ceiling and benefit from
    // native mode's stricter formatting guarantees.
    let force_text_mode = agent_id == "integrations_agent" && !tool_specs.is_empty();

    let supports_native =
        !force_text_mode && provider.supports_native_tools() && !tool_specs.is_empty();
    let request_tools = if supports_native {
        Some(tool_specs)
    } else {
        None
    };

    if force_text_mode {
        // Append the XML tool protocol + available-tool list to the
        // existing system prompt. `history[0]` is the system message
        // built by `run_typed_mode` / `run_fork_mode` upstream; we
        // augment it in-place so the model learns the call format for
        // this session without an extra message round-trip.
        if let Some(sys) = history.iter_mut().find(|m| m.role == "system") {
            sys.content.push_str("\n\n");
            sys.content
                .push_str(&build_text_mode_tool_instructions(tool_specs));
        }
        tracing::info!(
            task_id = %task_id,
            agent_id = %agent_id,
            tool_count = tool_specs.len(),
            "[subagent_runner:text-mode] omitting tools from API request, injected XML tool protocol into system prompt"
        );
    }

    let mut usage = AggregatedUsage::default();

    // Per-iteration transcript persistence. Mirrors the main-agent
    // turn loop: right after each provider response lands (and again
    // after the final response is pushed) we flush the full history
    // to disk. A crash during tool execution no longer erases the
    // sub-agent's response — the bytes are on disk before any tool
    // runs. Best-effort: write failures are logged at `debug` and the
    // loop continues.
    let persist_transcript = |history: &[ChatMessage], usage: &AggregatedUsage| {
        let path = match transcript::resolve_keyed_transcript_path(
            &parent.workspace_dir,
            &transcript_stem,
        ) {
            Ok(p) => p,
            Err(err) => {
                tracing::debug!(
                    agent_id = %agent_id,
                    error = %err,
                    "[subagent_runner] failed to resolve transcript path"
                );
                return;
            }
        };
        let now = chrono::Utc::now().to_rfc3339();
        let meta = transcript::TranscriptMeta {
            agent_name: agent_id.to_string(),
            dispatcher: "native".into(),
            created: now.clone(),
            updated: now,
            turn_count: 1,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            charged_amount_usd: usage.charged_amount_usd,
        };
        if let Err(err) = transcript::write_transcript(&path, history, &meta, None) {
            tracing::debug!(
                agent_id = %agent_id,
                error = %err,
                "[subagent_runner] failed to write transcript"
            );
        }
    };

    for iteration in 0..max_iterations {
        tracing::debug!(
            task_id = %task_id,
            agent_id = %agent_id,
            iteration,
            history_len = history.len(),
            "[subagent_runner] iteration start"
        );

        let resp = provider
            .chat(
                ChatRequest {
                    messages: history.as_slice(),
                    tools: request_tools,
                    stream: None,
                },
                model,
                temperature,
            )
            .await?;

        if let Some(ref u) = resp.usage {
            usage.input_tokens += u.input_tokens;
            usage.output_tokens += u.output_tokens;
            usage.cached_input_tokens += u.cached_input_tokens;
            usage.charged_amount_usd += u.charged_amount_usd;
        }

        let response_text = resp.text.clone().unwrap_or_default();

        // In text mode the model emits `<tool_call>{…}</tool_call>` tags
        // inline inside `resp.text` (and `resp.tool_calls` is empty
        // because we told the provider not to structure them). Parse
        // them ourselves via the shared harness helper and synthesise a
        // `ToolCall` per parsed block so the rest of the loop can stay
        // uniform.
        let native_calls: Vec<ToolCall> = if force_text_mode {
            let (_cleaned, parsed) = super::super::parse::parse_tool_calls(&response_text);
            parsed
                .into_iter()
                .enumerate()
                .map(|(i, call)| {
                    let args_str = if call.arguments.is_null() {
                        "{}".to_string()
                    } else {
                        call.arguments.to_string()
                    };
                    ToolCall {
                        id: call
                            .id
                            .clone()
                            .unwrap_or_else(|| format!("call_text_{iteration}_{i}")),
                        name: call.name,
                        arguments: args_str,
                    }
                })
                .collect()
        } else {
            resp.tool_calls.clone()
        };

        if native_calls.is_empty() {
            tracing::debug!(
                task_id = %task_id,
                agent_id = %agent_id,
                iteration,
                final_chars = response_text.chars().count(),
                "[subagent_runner] no tool calls — returning final response"
            );
            history.push(ChatMessage::assistant(response_text.clone()));
            // Persist the final response before returning so the
            // transcript always captures the last provider reply.
            persist_transcript(history, &usage);
            return Ok((response_text, iteration + 1, usage));
        }

        // Persist the assistant turn. In native mode use the canonical
        // serialiser (wraps text + structured tool_calls for the
        // backend's jinja template). In text mode the raw response
        // already contains the `<tool_call>` tags inline, so persist it
        // verbatim — on the next turn the model sees its own prior
        // emissions exactly as it wrote them.
        if force_text_mode {
            history.push(ChatMessage::assistant(response_text.clone()));
        } else {
            let assistant_history_content =
                super::super::parse::build_native_assistant_history(&response_text, &native_calls);
            history.push(ChatMessage::assistant(assistant_history_content));
        }

        // Persist the assistant response + tool-call intents **before**
        // executing tools. If the session crashes mid-tool-call we
        // still have what the model emitted on disk.
        persist_transcript(history, &usage);

        // Execute each call, collect outputs. Native mode pushes one
        // `role=tool` message per call with the structured `tool_call_id`
        // reference. Text mode has no such reference (the model just
        // emitted tags in prose), so we batch all results into a single
        // user message formatted with `<tool_result>` tags — mirroring
        // XmlToolDispatcher's `format_results`.
        let mut text_mode_result_block = String::new();
        for call in &native_calls {
            let result_text = if !allowed_names.contains(&call.name) {
                tracing::warn!(
                    task_id = %task_id,
                    agent_id = %agent_id,
                    tool = %call.name,
                    "[subagent_runner] tool not in allowlist for this sub-agent"
                );
                format!(
                    "Error: tool '{}' is not available to the {} sub-agent",
                    call.name, agent_id
                )
            } else if let Some(tool) = extra_tools
                .iter()
                .find(|t| t.name() == call.name)
                .or_else(|| parent_tools.iter().find(|t| t.name() == call.name))
            {
                let args = parse_tool_arguments(&call.arguments);
                let timeout = crate::openhuman::tool_timeout::tool_execution_timeout_duration();
                match tokio::time::timeout(timeout, tool.execute(args)).await {
                    Ok(Ok(result)) => {
                        let raw = result.output();
                        if result.is_error {
                            format!("Error: {raw}")
                        } else {
                            raw
                        }
                    }
                    Ok(Err(err)) => format!("Error executing {}: {err}", call.name),
                    Err(_) => format!("Error: tool '{}' timed out", call.name),
                }
            } else {
                format!("Unknown tool: {}", call.name)
            };

            // Progressive-disclosure handoff: if this spawn has a cache
            // (integrations_agent-with-toolkit path) and the result is large
            // and not itself an error / not from the extractor tool,
            // stash the raw payload and replace it in history with a
            // short placeholder. The sub-agent can drill in with
            // `extract_from_result(result_id=..., query=...)` on the
            // next turn. Errors and already-extracted output go through
            // unchanged — no point handing off a 200-byte error or an
            // already-compressed summary.
            //
            // Cleaning happens before the size check so HTML-heavy tool
            // outputs (Gmail bodies, HTML-embedded Notion blocks) that
            // drop below threshold after stripping markup skip the
            // extract pipeline entirely. For anything still over
            // threshold, the cache stores the cleaned text — chunks see
            // real content, not `<div>` soup.
            let result_text = if let Some(cache) = handoff_cache {
                let skip_cleaning =
                    call.name == "extract_from_result" || result_text.starts_with("Error");
                let cleaned = if skip_cleaning {
                    result_text
                } else {
                    let pre_len = result_text.len();
                    let cleaned = clean_tool_output(&result_text);
                    if cleaned.len() < pre_len {
                        tracing::debug!(
                            tool = %call.name,
                            before_bytes = pre_len,
                            after_bytes = cleaned.len(),
                            saved_pct = ((pre_len - cleaned.len()) * 100) / pre_len.max(1),
                            "[subagent_runner:handoff] cleaned tool output (stripped markup/data-uris/whitespace)"
                        );
                    }
                    cleaned
                };
                let tokens = cleaned.len().div_ceil(4);
                if !skip_cleaning && tokens > HANDOFF_OVERSIZE_THRESHOLD_TOKENS {
                    let id = cache.store(call.name.clone(), cleaned.clone());
                    let placeholder = build_handoff_placeholder(&call.name, &id, &cleaned);
                    tracing::info!(
                        task_id = %task_id,
                        agent_id = %agent_id,
                        tool = %call.name,
                        raw_tokens = tokens,
                        raw_bytes = cleaned.len(),
                        threshold_tokens = HANDOFF_OVERSIZE_THRESHOLD_TOKENS,
                        result_id = %id,
                        "[subagent_runner:handoff] stashed oversized tool output; substituted placeholder into history"
                    );
                    placeholder
                } else {
                    cleaned
                }
            } else {
                result_text
            };

            if force_text_mode {
                let status = if result_text.starts_with("Error") {
                    "error"
                } else {
                    "ok"
                };
                let _ = std::fmt::Write::write_fmt(
                    &mut text_mode_result_block,
                    format_args!(
                        "<tool_result name=\"{}\" status=\"{}\">\n{}\n</tool_result>\n",
                        call.name, status, result_text
                    ),
                );
            } else {
                let tool_msg = serde_json::json!({
                    "tool_call_id": call.id,
                    "content": result_text,
                });
                history.push(ChatMessage::tool(tool_msg.to_string()));
            }
        }

        if force_text_mode && !text_mode_result_block.is_empty() {
            history.push(ChatMessage::user(format!(
                "[Tool results]\n{text_mode_result_block}"
            )));
        }

        // Persist again after tool results have been appended so the
        // on-disk transcript reflects each round's complete
        // assistant-intent + tool-result pair. Without this, a crash
        // between `persist_transcript` at line ~1044 and the next
        // iteration's provider call would leave the transcript without
        // the tool outputs the next turn will be reasoning from.
        persist_transcript(history, &usage);
    }

    Err(SubagentRunError::MaxIterationsExceeded(max_iterations))
}

fn parse_tool_arguments(arguments: &str) -> serde_json::Value {
    serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
