//! Sub-agent execution runner.
//!
//! Given an [`AgentDefinition`] and a task prompt, this module:
//!
//! 1. Reads the [`ParentExecutionContext`] task-local set by the parent
//!    [`crate::openhuman::agent::Agent::turn`].
//! 2. Resolves the sub-agent's model name (inherit / hint / exact).
//! 3. Filters the parent's tool registry per `definition.tools`,
//!    `disallowed_tools`, and `skill_filter` (or, in `fork` mode,
//!    inherits the parent's tools verbatim).
//! 4. Builds a narrow system prompt that strips the sections the
//!    definition asks to omit (`omit_identity`, `omit_memory_context`,
//!    `omit_safety_preamble`, `omit_skills_catalog`).
//! 5. Runs a slim inner tool-call loop using the parent's
//!    [`crate::openhuman::providers::Provider`] and returns a single
//!    text result. The intra-sub-agent history never leaks back to the
//!    parent — the parent only sees one compact tool result.
//!
//! Token-saving levers in this runner:
//! - **Narrow prompt** — typed sub-agents skip identity/memory/skills.
//! - **Filtered tools** — fewer schemas in the request body.
//! - **Cheaper model** — archetype hint usually selects a smaller model.
//! - **Lower max iterations** — definition-controlled per archetype.
//! - **No memory recall** — sub-agents skip per-turn memory loading entirely;
//!   the parent has already injected the relevant context.
//! - **Structural compaction** — sub-agent's tool-call history collapses
//!   into a single tool result block in the parent's history.
//! - **Fork-mode prefix replay** — `uses_fork_context` definitions
//!   replay the parent's exact bytes for backend prefix-cache hits.

use super::definition::{AgentDefinition, PromptSource, ToolScope};
use super::fork_context::{current_fork, current_parent, ForkContext, ParentExecutionContext};
use super::session::transcript;
use crate::openhuman::context::prompt::{
    render_subagent_system_prompt, PromptContext, PromptTool, SubagentRenderOptions,
};
use crate::openhuman::providers::{ChatMessage, ChatRequest, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolCategory, ToolResult, ToolSpec};
use async_trait::async_trait;
use futures::stream::StreamExt;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex as StdMutex};
use std::time::{Duration, Instant};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Per-spawn options that override or augment what the
/// [`AgentDefinition`] specifies. Built by `SpawnSubagentTool::execute`
/// from the parent model's call arguments.
#[derive(Debug, Clone, Default)]
pub struct SubagentRunOptions {
    /// Optional skill-id override (e.g. `"notion"`). When set, the
    /// resolved tool list is further restricted to tools whose name
    /// starts with `{skill}__`. Overrides `definition.skill_filter`.
    pub skill_filter_override: Option<String>,

    /// Optional category override. When set, replaces
    /// `definition.category_filter` for this single spawn. Useful when
    /// the parent wants to reuse a generic definition but scope it to
    /// skill or system tools for this specific call.
    pub category_filter_override: Option<ToolCategory>,

    /// Optional Composio toolkit scope (e.g. `"gmail"`, `"notion"`).
    /// When set, skill-category tools are further restricted to those
    /// whose name starts with the uppercased `{toolkit}_` prefix, and
    /// the sub-agent's rendered `Connected Integrations` section is
    /// narrowed to only that toolkit's entry. Used by main/orchestrator
    /// when spawning `skills_agent` for a specific platform so the
    /// sub-agent only sees one integration's tool catalogue.
    pub toolkit_override: Option<String>,

    /// Optional context blob the parent wants to inject before the
    /// task prompt. Rendered as a `[Context]\n…\n` prefix.
    pub context: Option<String>,

    /// Stable id for tracing / DomainEvents (defaults to a UUID).
    pub task_id: Option<String>,
}

/// Outcome of a single sub-agent run, returned to the parent.
#[derive(Debug, Clone)]
pub struct SubagentRunOutcome {
    /// Unique identifier for this sub-task run.
    pub task_id: String,
    /// The ID of the agent archetype used (e.g., `researcher`).
    pub agent_id: String,
    /// The final text response produced by the sub-agent.
    pub output: String,
    /// How many LLM round-trips were performed during the run.
    pub iterations: usize,
    /// Total wall-clock duration of the run.
    pub elapsed: Duration,
    /// Which execution mode was used (Typed vs. Fork).
    pub mode: SubagentMode,
}

/// Which prompt-construction path the runner took for a sub-agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentMode {
    /// Built a narrow, archetype-specific prompt with filtered tools.
    Typed,
    /// Replayed the parent's exact rendered prompt and history prefix.
    Fork,
}

impl SubagentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Fork => "fork",
        }
    }
}

/// Errors the runner can surface to the parent. The parent receives a
/// stringified version inside a tool result block.
#[derive(Debug, Error)]
pub enum SubagentRunError {
    #[error("spawn_subagent called outside of an agent turn — no parent context available")]
    NoParentContext,

    #[error(
        "fork-mode sub-agent requested but no ForkContext is set on the task-local. \
         Did the parent agent forget to call `Agent::turn` with fork support?"
    )]
    NoForkContext,

    #[error("agent definition '{0}' not found in registry")]
    DefinitionNotFound(String),

    #[error("failed to load archetype prompt from '{path}': {source}")]
    PromptLoad {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("provider call failed: {0}")]
    Provider(#[from] anyhow::Error),

    #[error("sub-agent exceeded maximum iterations ({0})")]
    MaxIterationsExceeded(usize),
}

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

    let outcome = if definition.uses_fork_context {
        let fork = current_fork().ok_or(SubagentRunError::NoForkContext)?;
        run_fork_mode(definition, task_prompt, &options, &parent, &fork, &task_id).await?
    } else {
        run_typed_mode(definition, task_prompt, &options, &parent, &task_id).await?
    };

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
    let category_filter = options
        .category_filter_override
        .or(definition.category_filter);
    let toolkit_filter = options.toolkit_override.as_deref();
    let mut allowed_indices = filter_tool_indices(
        &parent.all_tools,
        &definition.tools,
        &definition.disallowed_tools,
        options
            .skill_filter_override
            .as_deref()
            .or(definition.skill_filter.as_deref()),
        category_filter,
    );

    // ── Force-include extra_tools that bypass category_filter ──────────
    //
    // `extra_tools` lets an agent definition request specific system tools
    // even when `category_filter` restricts to a different category. For
    // example, `skills_agent` sets `category_filter = "skill"` but still
    // needs `file_write` and `csv_export` for exporting oversized payloads.
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
            {
                allowed_indices.push(i);
            }
        }
    }

    // ── Dynamic per-action toolkit tools (skills_agent + toolkit) ──────
    //
    // When `skills_agent` is spawned with a `toolkit` argument (e.g.
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
    let is_skills_agent_with_toolkit = definition.id == "skills_agent" && toolkit_filter.is_some();
    if is_skills_agent_with_toolkit {
        // Drop EVERY skill-category parent tool. In the new
        // architecture all integration discovery / authorization /
        // dispatching is the orchestrator's responsibility (via the
        // Delegation Guide and `spawn_subagent` pre-flight). The
        // sub-agent's only job is to execute per-action tools for
        // its bound toolkit, so leftover meta-tools (composio_*,
        // apify_*, other-toolkit dispatchers) are pure noise that
        // confuses the model and wastes tokens.
        allowed_indices.retain(|&i| parent.all_tools[i].category() != ToolCategory::Skill);

        if let (Some(tk), Some(client)) = (toolkit_filter, parent.composio_client.as_ref()) {
            // The spawn_subagent pre-flight already verified the
            // toolkit is in the allowlist AND has an active
            // connection, so the matching entry must be present and
            // marked connected. Defensive lookup anyway.
            if let Some(integration) = parent
                .connected_integrations
                .iter()
                .find(|ci| ci.connected && ci.toolkit.eq_ignore_ascii_case(tk))
            {
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
                let filter_hits = super::tool_filter::filter_actions_by_prompt(
                    task_prompt,
                    &integration.tools,
                    top_k,
                );
                let selected: Vec<&crate::openhuman::context::prompt::ConnectedIntegrationTool> =
                    if filter_hits.len() >= super::tool_filter::MIN_CONFIDENT_HITS {
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
    // Built only for skills_agent-with-toolkit because that's the only
    // typed sub-agent that regularly calls external tools capable of
    // returning megabyte-scale payloads (Composio actions). Every other
    // typed sub-agent gets `None` and its tool results stay inline.
    //
    // When enabled, oversized tool results get stashed into this cache
    // and their place in history is taken by a short placeholder (see
    // `build_handoff_placeholder`). The sub-agent can then call the
    // companion `extract_from_result` tool below to dispatch the
    // summarizer sub-agent against the cached payload with a targeted
    // query. Lazy / pay-per-question, so trivial asks answerable from
    // the preview don't pay any extra LLM cost.
    let handoff_cache: Option<Arc<ResultHandoffCache>> = if is_skills_agent_with_toolkit {
        let cache = Arc::new(ResultHandoffCache::new());

        // Resolve the summarizer definition once and register the
        // extract_from_result tool alongside the composio action tools.
        // If the summarizer isn't in the registry (shouldn't happen in
        // production but can happen in tests), skip the tool — tool
        // results will still get the placeholder + preview, the
        // sub-agent just won't be able to drill in.
        if let Some(reg) =
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
        {
            if let Some(summarizer_def) = reg.get("summarizer") {
                dynamic_tools.push(Box::new(ExtractFromResultTool::new(
                    cache.clone(),
                    summarizer_def.clone(),
                )));
                tracing::debug!(
                    agent_id = %definition.id,
                    "[subagent_runner:typed] registered extract_from_result tool + handoff cache"
                );
            } else {
                tracing::warn!(
                    agent_id = %definition.id,
                    "[subagent_runner:typed] summarizer definition missing from registry — extract_from_result disabled (oversized results will still be cached and previewed)"
                );
            }
        }
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
    // definitions like `code_executor` / `tool_maker` / `skills_agent`
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
    let empty_visible: std::collections::HashSet<String> = std::collections::HashSet::new();
    let prompt_ctx = PromptContext {
        workspace_dir: &parent.workspace_dir,
        model_name: &model,
        agent_id: &definition.id,
        tools: &prompt_tools,
        skills: &parent.skills,
        dispatcher_instructions: "",
        learned: crate::openhuman::context::prompt::LearnedContextData::default(),
        visible_tool_names: &empty_visible,
        tool_call_format: parent.tool_call_format,
        connected_integrations: &narrowed_integrations,
        include_profile: !definition.omit_profile,
        include_memory_md: !definition.omit_memory_md,
    };

    let system_prompt = match &definition.system_prompt {
        PromptSource::Dynamic(build) => {
            // Function-driven builder returns the final prompt text.
            build(&prompt_ctx).map_err(|e| SubagentRunError::PromptLoad {
                path: format!("<dynamic:{}>", definition.id),
                source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
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
    let mut context_parts: Vec<&str> = Vec::new();
    if !definition.omit_memory_context {
        if let Some(ref mem_ctx) = parent.memory_context {
            context_parts.push(mem_ctx);
        }
    }
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
    let (output, iterations, agg_usage) = run_inner_loop(
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
    )
    .await?;

    persist_subagent_transcript(&parent.workspace_dir, &definition.id, &history, &agg_usage);

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
    let allowed_names: HashSet<String> = parent
        .all_tools
        .iter()
        .map(|t| t.name().to_string())
        .collect();

    let model = parent.model_name.clone();
    let temperature = parent.temperature;
    // Use the parent's iteration cap, not the synthetic fork definition's.
    let max_iterations = parent.agent_config.max_tool_iterations.max(1);

    // Fork mode replays the parent's exact tool list — no dynamic
    // toolkit-scoped tools, so `extra_tools` is empty.
    let fork_extra_tools: Vec<Box<dyn Tool>> = Vec::new();
    let (output, iterations, agg_usage) = run_inner_loop(
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
    )
    .await?;

    persist_subagent_transcript(&parent.workspace_dir, &definition.id, &history, &agg_usage);

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
// Session transcript persistence for sub-agents
// ─────────────────────────────────────────────────────────────────────────────

/// Best-effort: persist the sub-agent's conversation as a session transcript
/// so it can be inspected for debugging and KV cache analysis.
fn persist_subagent_transcript(
    workspace_dir: &Path,
    agent_id: &str,
    history: &[ChatMessage],
    usage: &AggregatedUsage,
) {
    let path = match transcript::resolve_new_transcript_path(workspace_dir, agent_id) {
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
    } else {
        tracing::debug!(
            agent_id = %agent_id,
            messages = history.len(),
            path = %path.display(),
            "[subagent_runner] transcript written"
        );
    }
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
) -> Result<(String, usize, AggregatedUsage), SubagentRunError> {
    let max_iterations = max_iterations.max(1);

    // ── Text-mode override for skills_agent ────────────────────────────
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
    // Scoped to `skills_agent` because that's the only path where we
    // pass Composio toolkit schemas. Every other typed sub-agent
    // (welcome, researcher, summarizer, …) uses small built-in tool
    // sets that stay well under the grammar ceiling and benefit from
    // native mode's stricter formatting guarantees.
    let force_text_mode = agent_id == "skills_agent" && !tool_specs.is_empty();

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
            let (_cleaned, parsed) = super::parse::parse_tool_calls(&response_text);
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
                super::parse::build_native_assistant_history(&response_text, &native_calls);
            history.push(ChatMessage::assistant(assistant_history_content));
        }

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
            // (skills_agent-with-toolkit path) and the result is large
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
    }

    Err(SubagentRunError::MaxIterationsExceeded(max_iterations))
}

fn parse_tool_arguments(arguments: &str) -> serde_json::Value {
    serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Oversized-tool-result handoff (progressive disclosure)
// ─────────────────────────────────────────────────────────────────────────────
//
// Typed sub-agents (skills_agent in particular) regularly call tools that
// return megabyte-scale payloads — `GMAIL_LIST_MESSAGES`, `NOTION_GET_PAGE`,
// `GOOGLEDRIVE_LIST_FILES`. The default behaviour pushes that raw blob into
// the sub-agent's history as a tool-result message, and the NEXT iteration
// then ships the bloated history back to the provider where it hits the
// model's context-length ceiling (observed: 276k-token prompt against a
// 196 607-token window → backend 400).
//
// The summarizer dispatches to the model with the payload + an extraction
// contract, but wiring it to fire on EVERY oversized result has two costs:
// (1) we pay for a summarizer turn even when the sub-agent could answer
// from a preview, and (2) aggressive compression sometimes drops the exact
// identifier the agent needs for a follow-up call.
//
// Progressive disclosure fixes both: when a tool returns too much data,
// we stash the full payload in a session-scoped cache, replace it in
// history with a short placeholder (size + preview + result_id + how to
// query it), and expose an `extract_from_result` tool that the sub-agent
// can call with a targeted query. The summarizer only runs when the
// sub-agent actually asks for a narrower view.

/// Token threshold above which a tool result is routed to the handoff
/// cache instead of being pushed into history raw. 20 000 tokens keeps
/// the sub-agent's per-iteration prompt well below the 196 607-token
/// model ceiling even after many iterations, and leaves comfortable
/// headroom for the system prompt + tool catalogue. Token count is
/// estimated at ~4 chars/token (mirrors
/// [`crate::openhuman::agent::harness::payload_summarizer`] and
/// [`crate::openhuman::tree_summarizer::types::estimate_tokens`]).
const HANDOFF_OVERSIZE_THRESHOLD_TOKENS: usize = 20_000;

/// Characters of the raw payload to surface in the placeholder preview.
/// Enough for the sub-agent to recognise the shape (JSON keys, first
/// record) and often small enough to answer trivial questions without a
/// follow-up `extract_from_result` call.
const HANDOFF_PREVIEW_CHARS: usize = 1500;

/// Maximum entries per session. Bounded to keep memory use predictable
/// on long-running sub-agents that might call many large tools. When
/// over capacity we evict the oldest entry (FIFO); callers see "no
/// cached result" for evicted ids and can either re-run the tool or
/// ask the user/orchestrator to narrow the request.
const HANDOFF_MAX_ENTRIES: usize = 8;

/// Per-spawn cache of oversized tool payloads. One instance is built
/// at the top of [`run_typed_mode`] and shared (via `Arc`) with both
/// the inner tool-call loop (writes) and the `extract_from_result`
/// tool (reads).
#[derive(Default)]
struct ResultHandoffCache {
    inner: StdMutex<HandoffInner>,
}

#[derive(Default)]
struct HandoffInner {
    /// FIFO of inserted ids, used for eviction.
    order: Vec<String>,
    /// Content by id.
    entries: HashMap<String, CachedResult>,
    /// Monotonic counter for id generation within the session.
    next_id: u64,
}

struct CachedResult {
    tool_name: String,
    content: String,
}

impl ResultHandoffCache {
    fn new() -> Self {
        Self::default()
    }

    /// Stash a payload and return a stable, short, grep-friendly id.
    fn store(&self, tool_name: String, content: String) -> String {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        g.next_id = g.next_id.saturating_add(1);
        let id = format!("res_{:x}", g.next_id);
        g.order.push(id.clone());
        g.entries
            .insert(id.clone(), CachedResult { tool_name, content });
        while g.order.len() > HANDOFF_MAX_ENTRIES {
            let evicted = g.order.remove(0);
            g.entries.remove(&evicted);
        }
        id
    }

    fn get(&self, result_id: &str) -> Option<CachedResult> {
        let g = self.inner.lock().ok()?;
        g.entries.get(result_id).map(|r| CachedResult {
            tool_name: r.tool_name.clone(),
            content: r.content.clone(),
        })
    }
}

/// Build the placeholder text that replaces an oversized tool result in
/// the sub-agent's history. Shows the payload size (estimated tokens
/// and raw bytes), a preview, and a call shape for the
/// `extract_from_result` tool. The sub-agent decides whether to answer
/// from the preview or dispatch the extractor.
///
/// Token count is estimated at ~4 chars/token (same heuristic as the
/// trigger threshold in `HANDOFF_OVERSIZE_THRESHOLD_TOKENS`), so the
/// unit the sub-agent sees matches the unit the runtime used to decide
/// to hand off in the first place.
fn build_handoff_placeholder(tool_name: &str, result_id: &str, raw: &str) -> String {
    let preview: String = raw.chars().take(HANDOFF_PREVIEW_CHARS).collect();
    let raw_tokens = raw.len().div_ceil(4);
    format!(
        "[oversized tool output: {raw_tokens} tokens ({raw_bytes} bytes) — stashed as result_id=\"{result_id}\"]\n\
         Preview (first {preview_chars} chars):\n{preview}\n\n\
         If the preview does not answer your task, call:\n\
         extract_from_result(result_id=\"{result_id}\", query=\"<specific question>\")\n\
         Good queries name the exact fields/identifiers you need \
         (e.g. \"subject and sender of the 5 most recent messages\"). \
         Tool: {tool_name}",
        raw_bytes = raw.len(),
        preview_chars = preview.chars().count(),
    )
}

/// Sub-agent-side tool that answers a targeted query against a payload
/// previously stashed via [`build_handoff_placeholder`]. Internally
/// dispatches the `summarizer` sub-agent with the cached payload + the
/// caller's query as the extraction contract.
struct ExtractFromResultTool {
    cache: Arc<ResultHandoffCache>,
    summarizer_def: AgentDefinition,
}

impl ExtractFromResultTool {
    fn new(cache: Arc<ResultHandoffCache>, summarizer_def: AgentDefinition) -> Self {
        Self {
            cache,
            summarizer_def,
        }
    }
}

#[async_trait]
impl Tool for ExtractFromResultTool {
    fn name(&self) -> &str {
        "extract_from_result"
    }

    fn description(&self) -> &str {
        "Answer a targeted question against an oversized tool output that was \
         stashed under a `result_id` handle. Use this when a previous tool call \
         returned a placeholder like `result_id=\"res_1\"` because its raw output \
         was too large to show inline. Pass the handle plus a natural-language \
         `query` naming the exact facts/identifiers you need; returns only the \
         extracted answer, not the full payload. Multiple queries against the \
         same `result_id` are allowed — each one is independent."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "result_id": {
                    "type": "string",
                    "description": "The handle emitted in the oversized tool output placeholder (e.g. `res_1`)."
                },
                "query": {
                    "type": "string",
                    "description": "Natural-language question naming the exact facts or identifiers to extract. Be specific."
                }
            },
            "required": ["result_id", "query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let result_id = args.get("result_id").and_then(|v| v.as_str()).unwrap_or("");
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");

        if result_id.is_empty() || query.is_empty() {
            return Ok(ToolResult::error(
                "extract_from_result requires non-empty `result_id` and `query`.",
            ));
        }

        let cached = match self.cache.get(result_id) {
            Some(c) => c,
            None => {
                return Ok(ToolResult::error(format!(
                    "No cached result found for id '{result_id}'. The handle may have been evicted (cache holds the {HANDOFF_MAX_ENTRIES} most recent entries). Re-run the original tool to get a fresh handle."
                )));
            }
        };

        // Fast path: payload fits in a single summarizer turn.
        if cached.content.len() <= SUMMARIZER_CHUNK_CHAR_BUDGET {
            tracing::debug!(
                tool = %cached.tool_name,
                bytes = cached.content.len(),
                "[extract_from_result] single-shot extraction"
            );
            return self
                .extract_single_shot(&cached.tool_name, &cached.content, query)
                .await;
        }

        // Slow path: chunk + parallel map. A single summarizer call on a
        // payload large enough to need the handoff (hundreds of KB
        // common for Gmail/Notion list operations) risks either (a)
        // overflowing the summarizer's own context window, or (b)
        // getting a low-quality single-pass summary that misses facts
        // near the tail. Splitting into budgeted chunks and running
        // them in parallel keeps each summarizer turn under its
        // context budget and usually finishes faster than a sequential
        // single-shot call on the whole blob.
        //
        // No reduce stage: per-chunk summaries are concatenated in
        // original chunk order. The reduce LLM call previously used
        // here added latency (often the slowest single turn of the
        // whole pipeline) and was the single point of failure when
        // the upstream provider hung — a hung reduce could burn
        // minutes before giving up. For listing/extraction queries
        // concatenation is equivalent; for top-N / global-ordering
        // queries the caller can post-process.
        let chunks = chunk_content(&cached.content, SUMMARIZER_CHUNK_CHAR_BUDGET);
        tracing::info!(
            tool = %cached.tool_name,
            total_bytes = cached.content.len(),
            chunk_count = chunks.len(),
            chunk_budget = SUMMARIZER_CHUNK_CHAR_BUDGET,
            "[extract_from_result] chunked extraction"
        );

        // Map stage: each chunk extracts items matching `query` from
        // ITS OWN slice only. Dispatched with bounded concurrency —
        // `buffer_unordered(MAP_CONCURRENCY)` keeps at most N summarizer
        // calls in flight at any time. Fully parallel `join_all` was
        // generating 504-gateway-timeout storms from the staging
        // proxy when 7+ concurrent summarizer calls piled onto the
        // upstream; batching at 3 trades some wall-clock time for
        // reliability. `run_subagent` is isolated per call (fresh
        // history, independent summarizer instance). Callers share
        // the same parent context.
        const MAP_CONCURRENCY: usize = 3;
        let total_chunks = chunks.len();

        // Consume `chunks` with `into_iter` so each async block owns
        // its `String` — `buffer_unordered` polls the stream lazily
        // and needs futures with no borrows into the enclosing scope.
        let map_futures = chunks.into_iter().enumerate().map(|(i, chunk)| {
            let summarizer_def = self.summarizer_def.clone();
            let tool_name = cached.tool_name.clone();
            let query = query.to_string();
            async move {
                let prompt = format!(
                    "Tool name: {tool_name}\nChunk {idx} of {total}\n\n\
                     Query: {query}\n\n\
                     This is one slice of a larger tool output. Extract ONLY \
                     items in THIS slice that match the query. Preserve \
                     identifiers verbatim (ids, urls, emails, timestamps). \
                     Return an empty string if nothing in this slice is \
                     relevant. Be concise — no preamble, no commentary on \
                     other slices.\n\n\
                     --- BEGIN SLICE ---\n{chunk}\n--- END SLICE ---",
                    idx = i + 1,
                    total = total_chunks,
                );
                (
                    i,
                    run_subagent(&summarizer_def, &prompt, SubagentRunOptions::default()).await,
                )
            }
        });

        let mut map_results: Vec<(usize, _)> = futures::stream::iter(map_futures)
            .buffer_unordered(MAP_CONCURRENCY)
            .collect()
            .await;
        // `buffer_unordered` yields futures in completion order; restore
        // original chunk order so the concatenated output matches the
        // natural ordering of the underlying tool result (e.g. Notion's
        // reverse-chrono page list).
        map_results.sort_by_key(|(i, _)| *i);

        let partials: Vec<String> = map_results
            .into_iter()
            .filter_map(|(i, r)| match r {
                Ok(outcome) => {
                    let trimmed = outcome.output.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        chunk_idx = i,
                        error = %e,
                        "[extract_from_result] map-stage summarizer call failed; dropping partial"
                    );
                    None
                }
            })
            .collect();

        if partials.is_empty() {
            return Ok(ToolResult::error(
                "extract_from_result: no matching content found across any chunk",
            ));
        }

        // Concatenate per-chunk summaries in original chunk order.
        // `join` with a single partial yields it unchanged (no trailing
        // separator), so no special-case is needed.
        Ok(ToolResult::success(partials.join("\n\n---\n\n")))
    }
}

impl ExtractFromResultTool {
    async fn extract_single_shot(
        &self,
        tool_name: &str,
        content: &str,
        query: &str,
    ) -> anyhow::Result<ToolResult> {
        let prompt = format!(
            "Tool name: {tool_name}\n\nQuery: {query}\n\n\
             Raw tool output — extract ONLY the information the query asks for. \
             Preserve identifiers verbatim (ids, urls, emails, timestamps). \
             Return a compact, direct answer, no preamble.\n\n\
             --- BEGIN ---\n{content}\n--- END ---",
        );

        match run_subagent(&self.summarizer_def, &prompt, SubagentRunOptions::default()).await {
            Ok(run) => {
                let trimmed = run.output.trim();
                if trimmed.is_empty() {
                    Ok(ToolResult::error(
                        "extract_from_result: summarizer returned an empty response",
                    ))
                } else {
                    Ok(ToolResult::success(trimmed.to_string()))
                }
            }
            Err(e) => Ok(ToolResult::error(format!(
                "extract_from_result: summarizer dispatch failed: {e}"
            ))),
        }
    }
}

/// Char budget per summarizer call. Chosen so a single chunk + prompt
/// scaffolding + output stays well below the summarization model's
/// context window (~196k tokens) — at ~4 chars/token that leaves
/// comfortable headroom for the extraction contract and response.
const SUMMARIZER_CHUNK_CHAR_BUDGET: usize = 60_000;

/// Split `content` into chunks no larger than `budget` bytes, breaking
/// at natural boundaries (blank lines, then single newlines) so the
/// summarizer rarely sees a structure torn mid-record. Falls back to
/// char-safe slicing for pathological single-line inputs.
/// Strip common noise from tool outputs before they're stashed or chunked.
///
/// Agent tools frequently return raw HTML email bodies, inline SVG, base64
/// data URIs, CSS/JS blocks, and collapsed whitespace — all of which bloat
/// the handoff cache and waste summarizer context on tokens that carry
/// zero semantic value for most extraction queries. Cleaning before the
/// oversize check means (a) some payloads drop below threshold entirely
/// and skip the extract pipeline, (b) chunked payloads fit more real
/// content per chunk, and (c) summarizers see clean text instead of
/// parsing around markup.
///
/// Applied generically to every tool output — no per-tool gating. The
/// patterns below target universally-noisy markup; non-HTML outputs
/// (plain JSON, plain text) are left essentially untouched because none
/// of these regexes match.
fn clean_tool_output(content: &str) -> String {
    // Block-level containers with entirely useless payloads — match lazily
    // across lines, case-insensitive. `(?s)` enables `.` matching `\n`.
    static SCRIPT_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<script\b[^>]*>.*?</script\s*>").unwrap());
    static STYLE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<style\b[^>]*>.*?</style\s*>").unwrap());
    static SVG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<svg\b[^>]*>.*?</svg\s*>").unwrap());
    static HTML_COMMENT_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
    // `data:<mime>;base64,<...>` inline payloads — keep the agent aware
    // an image/asset was there, but drop the bytes.
    static DATA_URI_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)data:[a-z0-9.+\-/]+;base64,[A-Za-z0-9+/=]+").unwrap());
    // Everything remaining that still looks like an HTML tag — strip the
    // tag but keep the text content. Deliberately greedy across attributes
    // but not across `>` so we don't eat inter-tag content.
    static HTML_TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
    // Runs of whitespace → single space; collapse vertical bloat.
    static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\f\v]+").unwrap());
    static BLANK_LINE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

    let cleaned = SCRIPT_RE.replace_all(content, "");
    let cleaned = STYLE_RE.replace_all(&cleaned, "");
    let cleaned = SVG_RE.replace_all(&cleaned, "[svg]");
    let cleaned = HTML_COMMENT_RE.replace_all(&cleaned, "");
    let cleaned = DATA_URI_RE.replace_all(&cleaned, "[data-uri]");
    let cleaned = HTML_TAG_RE.replace_all(&cleaned, "");
    let cleaned = WS_RE.replace_all(&cleaned, " ");
    let cleaned = BLANK_LINE_RE.replace_all(&cleaned, "\n\n");
    cleaned.trim().to_string()
}

fn chunk_content(content: &str, budget: usize) -> Vec<String> {
    if content.len() <= budget {
        return vec![content.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::with_capacity(budget.min(content.len()));

    let flush = |current: &mut String, chunks: &mut Vec<String>| {
        if !current.is_empty() {
            chunks.push(std::mem::take(current));
        }
    };

    for line in content.lines() {
        let projected = current.len() + line.len() + 1;
        if projected > budget && !current.is_empty() {
            flush(&mut current, &mut chunks);
        }
        if line.len() > budget {
            // Single line exceeds budget (e.g. JSON with no formatting).
            // Emit any pending content, then slice the line at char
            // boundaries so we don't panic on multi-byte chars.
            flush(&mut current, &mut chunks);
            let mut remaining = line;
            while !remaining.is_empty() {
                let mut cut = budget.min(remaining.len());
                while cut > 0 && !remaining.is_char_boundary(cut) {
                    cut -= 1;
                }
                if cut == 0 {
                    // Degenerate case — shouldn't happen for normal
                    // text. Take the entire remaining line to avoid
                    // an infinite loop.
                    chunks.push(remaining.to_string());
                    break;
                }
                chunks.push(remaining[..cut].to_string());
                remaining = &remaining[cut..];
            }
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    flush(&mut current, &mut chunks);

    chunks
}

/// Tight top-K ceiling for toolkits whose per-action JSON schemas are
/// dense enough to blow through either Fireworks' 65 535-rule grammar
/// cap (native mode) or the 196 607-token context cap (text mode) even
/// before any tool results land in history. Determined empirically from
/// the fixture dumps under `tests/fixtures/composio_*.json` and real
/// staging failures — see the trace where Gmail at top-K=25 produced
/// a 276k-token iter-1 prompt.
const HEAVY_SCHEMA_TOOLKITS: &[&str] = &[
    "gmail",
    "notion",
    "github",
    "salesforce",
    "hubspot",
    "googledrive",
    "googlesheets",
    "googledocs",
    "microsoftteams",
];

const TOOL_FILTER_TOP_K_DEFAULT: usize = 25;
const TOOL_FILTER_TOP_K_HEAVY: usize = 12;

/// Pick a top-K budget for the fuzzy filter based on how dense the
/// toolkit's action schemas tend to be. Match is case-insensitive so
/// we don't care whether the caller passed `"Gmail"` or `"gmail"`.
fn top_k_for_toolkit(toolkit: &str) -> usize {
    if HEAVY_SCHEMA_TOOLKITS
        .iter()
        .any(|t| t.eq_ignore_ascii_case(toolkit))
    {
        TOOL_FILTER_TOP_K_HEAVY
    } else {
        TOOL_FILTER_TOP_K_DEFAULT
    }
}

/// Format a set of `ToolSpec`s as an XML tool-use protocol block
/// appended to the system prompt in text mode. Mirrors
/// [`crate::openhuman::agent::dispatcher::XmlToolDispatcher::prompt_instructions`]
/// — same `<tool_call>{…}</tool_call>` format so the existing
/// `parse_tool_calls` helper understands what the model emits.
///
/// Per-parameter rendering is intentionally **compact**: name, type, a
/// "required" marker, and a short one-line description if present. We
/// do **not** serialise the full JSON schema. Composio/Fireworks action
/// schemas for toolkits like Gmail or Notion run multiple KB each —
/// embedding them verbatim blows up the prompt past the model's
/// context window (282k+ tokens for 26 Gmail tools vs a 196k cap).
/// The compact listing keeps the model informed enough to call tools
/// correctly while staying within budget. If the model needs deeper
/// schema detail it can surface the error and the orchestrator will
/// clarify on the next turn.
fn build_text_mode_tool_instructions(specs: &[ToolSpec]) -> String {
    let mut out = String::new();
    out.push_str("## Tool Use Protocol\n\n");
    out.push_str(
        "To use a tool, wrap a JSON object in <tool_call></tool_call> tags. \
         Do not nest tags. Emit one tag per call; you can emit multiple tags \
         in the same response if you need to run calls in parallel.\n\n",
    );
    out.push_str(
        "```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n\n",
    );
    out.push_str("### Available Tools\n\n");
    for spec in specs {
        let _ = writeln!(
            out,
            "- **{}**: {}",
            spec.name,
            first_line_truncated(&spec.description, 120)
        );
        let params_line = summarise_parameters(&spec.parameters);
        if !params_line.is_empty() {
            let _ = writeln!(out, "  Parameters: {}", params_line);
        }
    }
    out
}

/// Render a JSON-schema `parameters` object as a single-line,
/// ultra-compact parameter list — `*name: type, optName: type` for each
/// top-level property (leading `*` marks required). Deeply nested
/// shapes, enums, descriptions, and examples are all dropped.
///
/// Kept intentionally terse: Composio action schemas routinely contain
/// per-parameter descriptions several hundred tokens long, so even a
/// "short description" per param balloons to tens of thousands of
/// tokens across a 27-tool skills_agent toolkit and pushes the prompt
/// past the 196 607-token context window. The model can infer usage
/// from the parameter names + the tool's overall description; any
/// validation mismatch surfaces at call time and the orchestrator can
/// course-correct on the next turn.
fn summarise_parameters(params: &serde_json::Value) -> String {
    let Some(props) = params.get("properties").and_then(|v| v.as_object()) else {
        return String::new();
    };
    let required: std::collections::HashSet<&str> = params
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut parts: Vec<String> = Vec::with_capacity(props.len());
    for (name, schema) in props {
        let ty = schema.get("type").and_then(|v| v.as_str()).unwrap_or("any");
        let marker = if required.contains(name.as_str()) {
            "*"
        } else {
            ""
        };
        parts.push(format!("{marker}{name}:{ty}"));
    }
    parts.join(", ")
}

/// Return the first line of `s`, trimmed and truncated to `max_chars`
/// with a trailing ellipsis when it overflows. Used to keep
/// tool/parameter descriptions on a single grep-friendly line.
fn first_line_truncated(s: &str, max_chars: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    if first.chars().count() <= max_chars {
        first.to_string()
    } else {
        let truncated: String = first.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool filtering
// ─────────────────────────────────────────────────────────────────────────────

/// Returns indices into `parent_tools` for the tools the sub-agent may
/// invoke. Index-based filtering avoids cloning `Box<dyn Tool>` (which
/// isn't Clone) and lets us reuse the parent's existing instances.
///
/// Filters are applied in this order (shorter-circuit first):
/// 1. `disallowed` — explicit deny list.
/// 2. `category_filter` — restrict to `System` or `Skill` category.
/// 3. `skill_filter` — restrict to tools named `{skill}__*`.
/// 4. `scope` — `Wildcard` (everything remaining) or `Named` allowlist.
/// Filter a parent's tool registry down to the indices a sub-agent is
/// allowed to call, given its [`ToolScope`] + disallowed list +
/// skill/category filters.
///
/// Exposed `pub(crate)` so the debug dump path in
/// [`crate::openhuman::context::debug_dump`] shares the exact same
/// filter logic as the live runner — previously debug_dump carried a
/// "standalone copy" which drifted over time.
pub(crate) fn filter_tool_indices(
    parent_tools: &[Box<dyn Tool>],
    scope: &ToolScope,
    disallowed: &[String],
    skill_filter: Option<&str>,
    category_filter: Option<ToolCategory>,
) -> Vec<usize> {
    let disallow_set: HashSet<&str> = disallowed.iter().map(|s| s.as_str()).collect();
    let skill_prefix = skill_filter.map(|s| format!("{s}__"));

    parent_tools
        .iter()
        .enumerate()
        .filter(|(_, tool)| {
            let name = tool.name();
            if disallow_set.contains(name) {
                return false;
            }
            if let Some(required) = category_filter {
                if tool.category() != required {
                    return false;
                }
            }
            if let Some(prefix) = skill_prefix.as_deref() {
                if !name.starts_with(prefix) {
                    return false;
                }
            }
            match scope {
                ToolScope::Wildcard => true,
                ToolScope::Named(allowed) => allowed.iter().any(|n| n == name),
            }
        })
        .map(|(i, _)| i)
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Prompt loading + composition
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve a [`PromptSource`] to its raw markdown body. Inline sources
/// return immediately, `Dynamic` calls the builder with the supplied
/// [`PromptContext`], `File` sources are read from disk relative to the
/// workspace `prompts/` directory or the agent crate's bundled prompts.
///
/// Exposed `pub(crate)` so the debug dump path in
/// [`crate::openhuman::context::debug_dump`] loads prompts through the
/// exact same code the runner uses — no parallel body-loading logic.
pub(crate) fn load_prompt_source(
    source: &PromptSource,
    ctx: &PromptContext<'_>,
) -> Result<String, SubagentRunError> {
    let workspace_dir = ctx.workspace_dir;
    match source {
        PromptSource::Inline(body) => Ok(body.clone()),
        PromptSource::Dynamic(build) => build(ctx).map_err(|e| SubagentRunError::PromptLoad {
            path: format!("<dynamic:{}>", ctx.agent_id),
            source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
        }),
        PromptSource::File { path } => {
            // Try the workspace's `agent/prompts/` first (so users can
            // override built-in prompts), then fall back to the crate's
            // own bundled prompts via `include_str!`-style lookup.
            let workspace_path = workspace_dir.join("agent").join("prompts").join(path);
            if workspace_path.is_file() {
                return std::fs::read_to_string(&workspace_path).map_err(|e| {
                    SubagentRunError::PromptLoad {
                        path: workspace_path.display().to_string(),
                        source: e,
                    }
                });
            }
            // Built-in prompt fallback. The agent prompts directory is
            // already shipped at `src/openhuman/agent/prompts/` and
            // included in the binary via the `IdentitySection` workspace
            // file write — so we re-use that scaffolding by reading from
            // `<workspace>/<filename>` after the parent agent has
            // bootstrapped its workspace files. For sub-agent
            // archetype prompts (e.g. `archetypes/researcher.md`),
            // we look up by basename in the workspace, then accept
            // missing files as an empty body (the runner will fall
            // back to a generic role hint).
            let workspace_root_path = workspace_dir.join(path);
            if workspace_root_path.is_file() {
                return std::fs::read_to_string(&workspace_root_path).map_err(|e| {
                    SubagentRunError::PromptLoad {
                        path: workspace_root_path.display().to_string(),
                        source: e,
                    }
                });
            }
            tracing::warn!(
                path = %path,
                "[subagent_runner] archetype prompt file not found, using empty body"
            );
            Ok(String::new())
        }
    }
}

// Note: the narrow sub-agent prompt renderer lives in
// `crate::openhuman::context::prompt::render_subagent_system_prompt`
// so every system-prompt-building call-site — main agents, sub-agents,
// channel runtimes — shares one module.

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::definition::ModelSpec;
    use super::*;

    fn make_def_named_tools(names: &[&str]) -> AgentDefinition {
        AgentDefinition {
            id: "test".into(),
            when_to_use: "t".into(),
            display_name: None,
            system_prompt: PromptSource::Inline("system".into()),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            omit_skills_catalog: true,
            omit_profile: true,
            omit_memory_md: true,
            model: ModelSpec::Inherit,
            temperature: 0.4,
            tools: ToolScope::Named(names.iter().map(|s| s.to_string()).collect()),
            disallowed_tools: vec![],
            skill_filter: None,
            category_filter: None,
            extra_tools: vec![],
            max_iterations: 5,
            timeout_secs: None,
            sandbox_mode: super::super::definition::SandboxMode::None,
            background: false,
            uses_fork_context: false,
            subagents: vec![],
            delegate_name: None,
            source: super::super::definition::DefinitionSource::Builtin,
        }
    }

    /// Local tool used to populate `parent_tools` in tests.
    struct StubTool {
        name: &'static str,
    }

    use crate::openhuman::tools::{PermissionLevel, ToolResult};
    use async_trait::async_trait;

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::None
        }
    }

    fn stub(name: &'static str) -> Box<dyn Tool> {
        Box::new(StubTool { name })
    }

    #[test]
    fn filter_named_scope_keeps_only_named() {
        let parent: Vec<Box<dyn Tool>> = vec![stub("alpha"), stub("beta"), stub("gamma")];
        let def = make_def_named_tools(&["alpha", "gamma"]);
        let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, None, None);
        let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
        assert_eq!(names, vec!["alpha", "gamma"]);
    }

    #[test]
    fn filter_wildcard_includes_all_minus_disallowed() {
        let parent: Vec<Box<dyn Tool>> = vec![stub("alpha"), stub("beta"), stub("gamma")];
        let mut def = make_def_named_tools(&[]);
        def.tools = ToolScope::Wildcard;
        def.disallowed_tools = vec!["beta".into()];
        let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, None, None);
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
        let idx = filter_tool_indices(
            &parent,
            &def.tools,
            &def.disallowed_tools,
            Some("notion"),
            None,
        );
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
        let idx = filter_tool_indices(
            &parent,
            &def.tools,
            &def.disallowed_tools,
            Some("notion"),
            None,
        );
        let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
        assert_eq!(names, vec!["notion__search"]);
    }

    /// A stub tool that claims to be a skill-category tool, so we can
    /// exercise `filter_tool_indices` / `category_filter` without
    /// needing the real skill-bridge runtime.
    struct SkillStubTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for SkillStubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "skill stub"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::Write
        }
        fn category(&self) -> ToolCategory {
            ToolCategory::Skill
        }
    }

    fn skill_stub(name: &'static str) -> Box<dyn Tool> {
        Box::new(SkillStubTool { name })
    }

    #[test]
    fn filter_category_skill_keeps_only_skill_tools() {
        let parent: Vec<Box<dyn Tool>> = vec![
            stub("file_read"),
            stub("shell"),
            skill_stub("notion__search"),
            skill_stub("gmail__send"),
        ];
        let def = make_def_named_tools(&[]); // Named([])
                                             // Wildcard + Skill category → only skill-category tools.
        let mut def = def;
        def.tools = ToolScope::Wildcard;
        let idx = filter_tool_indices(
            &parent,
            &def.tools,
            &def.disallowed_tools,
            None,
            Some(ToolCategory::Skill),
        );
        let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
        assert_eq!(names, vec!["notion__search", "gmail__send"]);
    }

    #[test]
    fn filter_category_system_excludes_skill_tools() {
        let parent: Vec<Box<dyn Tool>> = vec![
            stub("file_read"),
            skill_stub("notion__search"),
            stub("shell"),
        ];
        let mut def = make_def_named_tools(&[]);
        def.tools = ToolScope::Wildcard;
        let idx = filter_tool_indices(
            &parent,
            &def.tools,
            &def.disallowed_tools,
            None,
            Some(ToolCategory::System),
        );
        let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
        assert_eq!(names, vec!["file_read", "shell"]);
    }

    #[test]
    fn filter_category_and_skill_filter_compose() {
        // Category=Skill AND skill_filter=notion → only notion__* tools
        // that are also Skill-category.
        let parent: Vec<Box<dyn Tool>> = vec![
            skill_stub("notion__search"),
            skill_stub("notion__read"),
            skill_stub("gmail__send"),
            stub("file_read"),
            // A pathological system-category tool with a "notion__"
            // name prefix — the category filter should still exclude it.
            stub("notion__fake"),
        ];
        let mut def = make_def_named_tools(&[]);
        def.tools = ToolScope::Wildcard;
        let idx = filter_tool_indices(
            &parent,
            &def.tools,
            &def.disallowed_tools,
            Some("notion"),
            Some(ToolCategory::Skill),
        );
        let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
        assert_eq!(names, vec!["notion__search", "notion__read"]);
    }

    /// End-to-end verification that a sub-agent with
    /// `category_filter = "skill"` (like the built-in `skills_agent`) sees
    /// the real Composio tools alongside any other `Skill`-category tools
    /// and does **not** see `System`-category tools.
    ///
    /// This is the regression test for "skills subagent has access to
    /// composio tools": if any of the composio tool impls forgets to
    /// override `category()` and falls back to the default `System`, it
    /// gets filtered out here and this test fails.
    #[test]
    fn skills_subagent_filter_admits_composio_tools() {
        use crate::openhuman::composio::client::ComposioClient;
        use crate::openhuman::composio::tools::{
            ComposioAuthorizeTool, ComposioExecuteTool, ComposioListConnectionsTool,
            ComposioListToolkitsTool, ComposioListToolsTool,
        };
        use crate::openhuman::integrations::IntegrationClient;
        use std::sync::Arc;

        // Build a throwaway composio client. The filter only touches
        // `Tool::name()` and `Tool::category()`, so no HTTP calls happen.
        let inner =
            IntegrationClient::new("http://127.0.0.1:0".to_string(), "test-token".to_string());
        let client = ComposioClient::new(Arc::new(inner));

        // Parent registry = the five real Composio tools + a couple of
        // plain system-category stubs. We expect exactly the composio
        // tools to survive the skills sub-agent's category filter.
        let parent: Vec<Box<dyn Tool>> = vec![
            Box::new(ComposioListToolkitsTool::new(client.clone())),
            Box::new(ComposioListConnectionsTool::new(client.clone())),
            Box::new(ComposioAuthorizeTool::new(client.clone())),
            Box::new(ComposioListToolsTool::new(client.clone())),
            Box::new(ComposioExecuteTool::new(client)),
            stub("file_read"),
            stub("shell"),
        ];

        // Mirror the skills_agent definition: wildcard tool scope,
        // category_filter = Skill, no skill_filter.
        let mut def = make_def_named_tools(&[]);
        def.tools = ToolScope::Wildcard;
        let idx = filter_tool_indices(
            &parent,
            &def.tools,
            &def.disallowed_tools,
            None,
            Some(ToolCategory::Skill),
        );

        let surviving: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();

        // All five composio tools must be present.
        for expected in &[
            "composio_list_toolkits",
            "composio_list_connections",
            "composio_authorize",
            "composio_list_tools",
            "composio_execute",
        ] {
            assert!(
                surviving.contains(expected),
                "skills sub-agent filter dropped composio tool `{}` — \
                 did someone remove the `category()` override? \
                 surviving = {:?}",
                expected,
                surviving,
            );
        }

        // System-category tools must be filtered out.
        assert!(!surviving.contains(&"file_read"));
        assert!(!surviving.contains(&"shell"));

        // And we should see exactly 5 survivors, no more, no less.
        assert_eq!(
            surviving.len(),
            5,
            "expected exactly 5 composio tools to pass the skills filter, \
             got {:?}",
            surviving,
        );
    }

    #[test]
    fn subagent_mode_as_str_roundtrip() {
        assert_eq!(SubagentMode::Typed.as_str(), "typed");
        assert_eq!(SubagentMode::Fork.as_str(), "fork");
    }

    // ── End-to-end runner tests with mock provider ────────────────────────

    use super::super::fork_context::{with_fork_context, with_parent_context};
    use crate::openhuman::providers::{
        ChatRequest as PChatRequest, ChatResponse, Provider, ToolCall,
    };
    use parking_lot::Mutex;
    use std::sync::Arc;

    /// Mock provider whose response queue can be inspected by the test
    /// to verify the bytes that arrive at the model.
    #[derive(Clone)]
    struct CapturedRequest {
        messages: Vec<crate::openhuman::providers::ChatMessage>,
        tool_count: usize,
    }

    struct ScriptedProvider {
        responses: Mutex<Vec<ChatResponse>>,
        captured: Mutex<Vec<CapturedRequest>>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses),
                captured: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("noop".into())
        }

        async fn chat(
            &self,
            request: PChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            self.captured.lock().push(CapturedRequest {
                messages: request.messages.to_vec(),
                tool_count: request.tools.map_or(0, |tools| tools.len()),
            });
            let mut q = self.responses.lock();
            if q.is_empty() {
                return Ok(ChatResponse {
                    text: Some(String::new()),
                    tool_calls: vec![],
                    usage: None,
                });
            }
            Ok(q.remove(0))
        }

        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    fn text_response(text: &str) -> ChatResponse {
        ChatResponse {
            text: Some(text.into()),
            tool_calls: vec![],
            usage: None,
        }
    }

    fn tool_response(name: &str, args: &str) -> ChatResponse {
        ChatResponse {
            text: Some(String::new()),
            tool_calls: vec![ToolCall {
                id: "call-1".into(),
                name: name.into(),
                arguments: args.into(),
            }],
            usage: None,
        }
    }

    /// Build a minimal `ParentExecutionContext` suitable for runner tests.
    /// Uses a no-op memory backend so we don't have to spin up a real one.
    fn make_parent(
        provider: Arc<dyn Provider>,
        tools: Vec<Box<dyn Tool>>,
    ) -> ParentExecutionContext {
        let tool_specs: Vec<crate::openhuman::tools::ToolSpec> =
            tools.iter().map(|t| t.spec()).collect();
        ParentExecutionContext {
            provider,
            all_tools: Arc::new(tools),
            all_tool_specs: Arc::new(tool_specs),
            model_name: "test-model".into(),
            temperature: 0.5,
            workspace_dir: std::env::temp_dir(),
            memory: noop_memory(),
            agent_config: crate::openhuman::config::AgentConfig::default(),
            skills: Arc::new(vec![]),
            memory_context: None,
            session_id: "test-session".into(),
            channel: "test".into(),
            connected_integrations: vec![],
            composio_client: None,
            tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::PFormat,
        }
    }

    fn noop_memory() -> Arc<dyn crate::openhuman::memory::Memory> {
        struct NoopMemory;
        #[async_trait]
        impl crate::openhuman::memory::Memory for NoopMemory {
            async fn store(
                &self,
                _key: &str,
                _content: &str,
                _category: crate::openhuman::memory::MemoryCategory,
                _session_id: Option<&str>,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn recall(
                &self,
                _query: &str,
                _limit: usize,
                _session_id: Option<&str>,
            ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
                Ok(vec![])
            }
            async fn get(
                &self,
                _key: &str,
            ) -> anyhow::Result<Option<crate::openhuman::memory::MemoryEntry>> {
                Ok(None)
            }
            async fn list(
                &self,
                _category: Option<&crate::openhuman::memory::MemoryCategory>,
                _session_id: Option<&str>,
            ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
                Ok(vec![])
            }
            async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
                Ok(true)
            }
            async fn count(&self) -> anyhow::Result<usize> {
                Ok(0)
            }
            async fn health_check(&self) -> bool {
                true
            }
            fn name(&self) -> &str {
                "noop"
            }
        }
        Arc::new(NoopMemory)
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
                    skill_filter_override: None,
                    category_filter_override: None,
                    toolkit_override: None,
                    context: None,
                    task_id: Some("t1".into()),
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
        parent.memory_context = Some("[Memory context]\n- prior fact: branch X failed\n".into());
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
                    skill_filter_override: Some("notion".into()),
                    category_filter_override: None,
                    toolkit_override: None,
                    context: None,
                    task_id: None,
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
        assert!(
            tool_msg.content.contains("not available"),
            "blocked tool should produce a 'not available' error message"
        );
    }

    #[tokio::test]
    async fn fork_mode_replays_parent_prefix_bytes() {
        // Construct a fake fork context with a known message prefix.
        // The runner should replay it byte-for-byte plus a single
        // appended user message carrying the fork directive.
        let provider = ScriptedProvider::new(vec![text_response("fork done")]);
        let parent = make_parent(provider.clone(), vec![stub("file_read"), stub("shell")]);

        let prefix = vec![
            crate::openhuman::providers::ChatMessage::system("PARENT_SYSTEM_PROMPT_BYTES"),
            crate::openhuman::providers::ChatMessage::user("first user msg"),
            crate::openhuman::providers::ChatMessage::assistant("parent assistant"),
        ];

        let fork = ForkContext {
            system_prompt: Arc::new("PARENT_SYSTEM_PROMPT_BYTES".into()),
            tool_specs: Arc::new(vec![parent.all_tool_specs[0].clone()]),
            message_prefix: Arc::new(prefix.clone()),
            fork_task_prompt: "ANALYSE THIS BRANCH".into(),
        };

        let def = super::super::builtin_definitions::fork_definition();

        let outcome = with_parent_context(parent, async move {
            with_fork_context(fork, async {
                run_subagent(
                    &def,
                    "ignored — fork uses fork_task_prompt",
                    SubagentRunOptions::default(),
                )
                .await
            })
            .await
        })
        .await
        .expect("fork runner should succeed");

        assert_eq!(outcome.mode, SubagentMode::Fork);
        assert_eq!(outcome.output, "fork done");

        // Verify the request that hit the provider replays the parent
        // prefix exactly and appends only the fork directive.
        let captured = provider.captured.lock();
        let first_call = &captured[0];
        assert_eq!(first_call.messages.len(), prefix.len() + 1);
        for (i, msg) in prefix.iter().enumerate() {
            assert_eq!(first_call.messages[i].role, msg.role);
            assert_eq!(first_call.messages[i].content, msg.content);
        }
        // The appended user message carries the fork directive.
        let appended = first_call.messages.last().unwrap();
        assert_eq!(appended.role, "user");
        assert_eq!(appended.content, "ANALYSE THIS BRANCH");
        assert_eq!(first_call.tool_count, 1);
    }

    #[tokio::test]
    async fn fork_mode_errors_when_no_fork_context() {
        let provider = ScriptedProvider::new(vec![text_response("unused")]);
        let parent = make_parent(provider, vec![stub("file_read")]);
        let def = super::super::builtin_definitions::fork_definition();

        let result = with_parent_context(parent, async {
            run_subagent(&def, "x", SubagentRunOptions::default()).await
        })
        .await;

        assert!(matches!(result, Err(SubagentRunError::NoForkContext)));
    }

    #[tokio::test]
    async fn runner_errors_outside_parent_context() {
        let def = make_def_named_tools(&[]);
        let result = run_subagent(&def, "x", SubagentRunOptions::default()).await;
        assert!(matches!(result, Err(SubagentRunError::NoParentContext)));
    }
}
