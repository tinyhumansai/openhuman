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
//! - **No memory recall** — sub-agents use a [`crate::openhuman::agent::memory_loader::NullMemoryLoader`].
//! - **Structural compaction** — sub-agent's tool-call history collapses
//!   into a single tool result block in the parent's history.
//! - **Fork-mode prefix replay** — `uses_fork_context` definitions
//!   replay the parent's exact bytes for backend prefix-cache hits.

use super::definition::{AgentDefinition, PromptSource, ToolScope};
use super::fork_context::{current_fork, current_parent, ForkContext, ParentExecutionContext};
use crate::openhuman::providers::{ChatMessage, ChatRequest, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolCategory, ToolSpec};
use std::collections::HashSet;
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

/// Outcome of a single sub-agent run, returned to
/// `SpawnSubagentTool::execute` for relay back to the parent.
#[derive(Debug, Clone)]
pub struct SubagentRunOutcome {
    pub task_id: String,
    pub agent_id: String,
    pub output: String,
    pub iterations: usize,
    pub elapsed: Duration,
    pub mode: SubagentMode,
}

/// Which prompt-construction path the runner took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentMode {
    /// Built a narrow prompt + filtered tools (the common case).
    Typed,
    /// Replayed the parent's exact prompt + tools + message prefix.
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

/// Run a sub-agent.
///
/// On success returns a [`SubagentRunOutcome`] whose `output` is the
/// final assistant text. On failure the error is suitable for stringifying
/// into a `tool_result` block — the parent agent will surface it to the
/// model and decide whether to retry or apologise to the user.
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
    // We compose the prompt inline rather than routing through
    // `SystemPromptBuilder::for_subagent` because the builder requires
    // a slice of `Box<dyn Tool>` and we only have indices into the
    // parent's vec (Box isn't Clone, so we can't build an owning
    // filtered slice cheaply). `render_subagent_system_prompt` mirrors
    // the builder's output for the narrow case.
    let system_prompt = render_subagent_system_prompt(
        &parent.workspace_dir,
        &model,
        &allowed_indices,
        &parent.all_tools,
        definition,
        &archetype_prompt_body,
    );

    // ── Build the user message (with optional context prefix) ──────────
    let user_message = if let Some(ctx) = options.context.as_deref() {
        format!("[Context]\n{ctx}\n\n{task_prompt}")
    } else {
        task_prompt.to_string()
    };

    let mut history: Vec<ChatMessage> = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(user_message),
    ];

    // ── Run the inner tool-call loop ───────────────────────────────────
    let (output, iterations) = run_inner_loop(
        parent.provider.as_ref(),
        &mut history,
        &parent.all_tools,
        &filtered_specs,
        &allowed_names,
        &model,
        temperature,
        definition.max_iterations,
        task_id,
        &definition.id,
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

    // All parent tools are allowed in fork mode (the whole point is
    // byte-identical tool schemas).
    let allowed_names: HashSet<String> = parent
        .all_tools
        .iter()
        .map(|t| t.name().to_string())
        .collect();

    let model = parent.model_name.clone();
    let temperature = parent.temperature;
    // Use the parent's iteration cap, not the synthetic fork definition's.
    let max_iterations = parent.agent_config.max_tool_iterations.max(1);

    let (output, iterations) = run_inner_loop(
        parent.provider.as_ref(),
        &mut history,
        &parent.all_tools,
        &parent.all_tool_specs,
        &allowed_names,
        &model,
        temperature,
        max_iterations,
        task_id,
        &definition.id,
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
    task_id: &str,
    agent_id: &str,
) -> Result<(String, usize), SubagentRunError> {
    let max_iterations = max_iterations.max(1);
    let supports_native = provider.supports_native_tools() && !tool_specs.is_empty();
    let request_tools = if supports_native {
        Some(tool_specs)
    } else {
        None
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
                    system_prompt_cache_boundary: None,
                },
                model,
                temperature,
            )
            .await?;

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
            return Ok((response_text, iteration + 1));
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

/// Render the sub-agent's system prompt by hand. We avoid going through
/// `SystemPromptBuilder::build` because that requires a slice of
/// `Box<dyn Tool>` and we already have indices into the parent's vec
/// (which would force an awkward Vec<&Box<dyn Tool>> intermediate).
///
/// `archetype_body` is the already-loaded archetype markdown — for
/// `PromptSource::Inline` this is the inline string, for
/// `PromptSource::File` this is the file contents loaded by
/// [`load_prompt_source`]. The caller resolves the source exactly
/// once and hands the body in, so this renderer works uniformly for
/// both definition shapes.
///
/// # KV cache stability
///
/// The rendered bytes MUST be a pure function of:
/// - the `archetype_body` (archetype role prompt)
/// - the filtered tool set (names, descriptions, schemas)
/// - the workspace directory
/// - the resolved model name
///
/// Anything that varies across invocations at the *same* call site (e.g.
/// `chrono::Local::now()`, hostnames, pids, turn counters) is forbidden
/// here. Repeat spawns of the same sub-agent within a session must
/// produce byte-identical system prompts so the inference backend's
/// automatic prefix caching can reuse the prefill from the previous run.
/// Time-of-day information, if a sub-agent needs it, belongs in the user
/// message — not the system prompt.
fn render_subagent_system_prompt(
    workspace_dir: &std::path::Path,
    model_name: &str,
    allowed_indices: &[usize],
    parent_tools: &[Box<dyn Tool>],
    definition: &AgentDefinition,
    archetype_body: &str,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = definition; // reserved for future per-definition rendering flags

    // 1. Archetype role prompt. Works for both `PromptSource::Inline`
    //    and `PromptSource::File` because the caller preloaded the
    //    body via `load_prompt_source`.
    let trimmed = archetype_body.trim();
    if !trimmed.is_empty() {
        out.push_str(trimmed);
        out.push_str("\n\n");
    }

    // 2. Filtered tool catalogue. Indices are taken in ascending order
    //    from `allowed_indices`, which itself preserves `parent_tools`
    //    order, so the rendering is deterministic.
    out.push_str("## Tools\n\n");
    for &i in allowed_indices {
        let tool = &parent_tools[i];
        let _ = writeln!(
            out,
            "- **{}**: {}\n  Parameters: `{}`",
            tool.name(),
            tool.description(),
            tool.parameters_schema()
        );
    }

    // 3. Sub-agent calling-convention preamble. Mirrors the existing
    //    NativeToolDispatcher hint that gets baked into the parent's
    //    prompt — sub-agents need it too.
    out.push('\n');
    out.push_str(
        "Use the provided tools to accomplish the task. Reply with a concise, dense \
                 final answer when you have one — the parent agent will weave it back into the \
                 user-visible response.\n\n",
    );

    // 4. Workspace so the model knows where it is. Intentionally stable:
    //    no datetime, no hostname, no pid — see the KV-cache note above.
    let _ = writeln!(
        out,
        "## Workspace\n\nWorking directory: `{}`\n",
        workspace_dir.display()
    );

    // 5. Runtime banner — model name only. Stable for the lifetime of
    //    this sub-agent's definition.
    let _ = writeln!(out, "## Runtime\n\nModel: {model_name}");

    out
}

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
    struct ScriptedProvider {
        responses: Mutex<Vec<ChatResponse>>,
        captured: Mutex<Vec<Vec<crate::openhuman::providers::ChatMessage>>>,
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
            self.captured.lock().push(request.messages.to_vec());
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
            identity_config: crate::openhuman::config::IdentityConfig::default(),
            skills: Arc::new(vec![]),
            session_id: "test-session".into(),
            channel: "test".into(),
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
        // Verifies that NullMemoryLoader is in effect: the user message
        // sent to the provider does NOT contain `[Memory context]`.
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
        let second_call_messages = &captured[1];
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
        let second_call_messages = &captured[1];
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
        let parent = make_parent(provider.clone(), vec![stub("file_read")]);

        let prefix = vec![
            crate::openhuman::providers::ChatMessage::system("PARENT_SYSTEM_PROMPT_BYTES"),
            crate::openhuman::providers::ChatMessage::user("first user msg"),
            crate::openhuman::providers::ChatMessage::assistant("parent assistant"),
        ];

        let fork = ForkContext {
            system_prompt: Arc::new("PARENT_SYSTEM_PROMPT_BYTES".into()),
            tool_specs: Arc::clone(&parent.all_tool_specs),
            message_prefix: Arc::new(prefix.clone()),
            cache_boundary: None,
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
        assert_eq!(first_call.len(), prefix.len() + 1);
        for (i, msg) in prefix.iter().enumerate() {
            assert_eq!(first_call[i].role, msg.role);
            assert_eq!(first_call[i].content, msg.content);
        }
        // The appended user message carries the fork directive.
        let appended = first_call.last().unwrap();
        assert_eq!(appended.role, "user");
        assert_eq!(appended.content, "ANALYSE THIS BRANCH");
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
