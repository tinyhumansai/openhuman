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
    extract_cache_boundary, render_subagent_system_prompt, SubagentRenderOptions,
};
use crate::openhuman::providers::{ChatMessage, ChatRequest, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolCategory, ToolSpec};
use std::collections::HashSet;
use std::path::Path;
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

    // ── Resolve archetype prompt body ──────────────────────────────────
    let archetype_prompt_body =
        load_prompt_source(&definition.system_prompt, &parent.workspace_dir)?;

    // ── Resolve model + temperature ────────────────────────────────────
    let model = definition.model.resolve(&parent.model_name);
    let temperature = definition.temperature;

    // ── Filter tools per definition + per-spawn override ───────────────
    let category_filter = options
        .category_filter_override
        .or(definition.category_filter);
    let allowed_indices = filter_tool_indices(
        &parent.all_tools,
        &definition.tools,
        &definition.disallowed_tools,
        options
            .skill_filter_override
            .as_deref()
            .or(definition.skill_filter.as_deref()),
        category_filter,
    );

    let filtered_specs: Vec<ToolSpec> = allowed_indices
        .iter()
        .map(|&i| parent.all_tool_specs[i].clone())
        .collect();
    let allowed_names: HashSet<String> = allowed_indices
        .iter()
        .map(|&i| parent.all_tools[i].name().to_string())
        .collect();

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
    let rendered_prompt = extract_cache_boundary(&render_subagent_system_prompt(
        &parent.workspace_dir,
        &model,
        &allowed_indices,
        &parent.all_tools,
        &archetype_prompt_body,
        render_options,
        &parent.connected_integrations,
    ));
    let system_prompt = rendered_prompt.text;
    let system_prompt_cache_boundary = rendered_prompt.cache_boundary;

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
        &filtered_specs,
        &allowed_names,
        &model,
        temperature,
        definition.max_iterations,
        system_prompt_cache_boundary,
        task_id,
        &definition.id,
    )
    .await?;

    persist_subagent_transcript(
        &parent.workspace_dir,
        &definition.id,
        &history,
        system_prompt_cache_boundary,
        &agg_usage,
    );

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
        cache_boundary = ?fork.cache_boundary,
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

    let (output, iterations, agg_usage) = run_inner_loop(
        parent.provider.as_ref(),
        &mut history,
        &parent.all_tools,
        fork.tool_specs.as_slice(),
        &allowed_names,
        &model,
        temperature,
        max_iterations,
        fork.cache_boundary,
        task_id,
        &definition.id,
    )
    .await?;

    persist_subagent_transcript(
        &parent.workspace_dir,
        &definition.id,
        &history,
        fork.cache_boundary,
        &agg_usage,
    );

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
    cache_boundary: Option<usize>,
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
        cache_boundary,
        created: now.clone(),
        updated: now,
        turn_count: 1,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        charged_amount_usd: usage.charged_amount_usd,
    };

    if let Err(err) = transcript::write_transcript(&path, history, &meta) {
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
    tool_specs: &[ToolSpec],
    allowed_names: &HashSet<String>,
    model: &str,
    temperature: f64,
    max_iterations: usize,
    system_prompt_cache_boundary: Option<usize>,
    task_id: &str,
    agent_id: &str,
) -> Result<(String, usize, AggregatedUsage), SubagentRunError> {
    let max_iterations = max_iterations.max(1);
    let supports_native = provider.supports_native_tools() && !tool_specs.is_empty();
    let request_tools = if supports_native {
        Some(tool_specs)
    } else {
        None
    };

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
                    system_prompt_cache_boundary,
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
        let native_calls: Vec<ToolCall> = resp.tool_calls.clone();

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

        // Persist assistant message with the original tool_calls payload so
        // subsequent role=tool messages can reference call ids correctly.
        let assistant_history_content =
            build_native_assistant_payload(&response_text, &native_calls);
        history.push(ChatMessage::assistant(assistant_history_content));

        // Execute each call, append role=tool messages.
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
            } else if let Some(tool) = parent_tools.iter().find(|t| t.name() == call.name) {
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

            let tool_msg = serde_json::json!({
                "tool_call_id": call.id,
                "content": result_text,
            });
            history.push(ChatMessage::tool(tool_msg.to_string()));
        }
    }

    Err(SubagentRunError::MaxIterationsExceeded(max_iterations))
}

fn build_native_assistant_payload(text: &str, tool_calls: &[ToolCall]) -> String {
    // Mirror the existing native-tool-call serialisation pattern used by
    // `agent::loop_::parse::build_native_assistant_history`. We inline a
    // small subset here to avoid an inter-module dep cycle.
    let calls_json: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|call| {
            serde_json::json!({
                "id": call.id,
                "type": "function",
                "function": {
                    "name": call.name,
                    "arguments": call.arguments,
                },
            })
        })
        .collect();

    let payload = serde_json::json!({
        "text": text,
        "tool_calls": calls_json,
    });
    payload.to_string()
}

fn parse_tool_arguments(arguments: &str) -> serde_json::Value {
    serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
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
fn filter_tool_indices(
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
/// return immediately; file sources are read from disk relative to the
/// workspace `prompts/` directory or the agent crate's bundled prompts.
fn load_prompt_source(
    source: &PromptSource,
    workspace_dir: &std::path::Path,
) -> Result<String, SubagentRunError> {
    match source {
        PromptSource::Inline(body) => Ok(body.clone()),
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
        cache_boundary: Option<usize>,
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
                cache_boundary: request.system_prompt_cache_boundary,
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
    async fn typed_mode_threads_system_prompt_cache_boundary() {
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
        assert!(
            captured[0].cache_boundary.is_some(),
            "typed sub-agent request should carry a prompt cache boundary"
        );
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
            cache_boundary: Some(9),
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
        assert_eq!(first_call.cache_boundary, Some(9));
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
