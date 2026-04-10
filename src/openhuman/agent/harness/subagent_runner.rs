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

use super::definition::{AgentDefinition, ModelSpec, PromptSource, ToolScope};
use super::fork_context::{current_fork, current_parent, ForkContext, ParentExecutionContext};
use crate::openhuman::agent::prompt::SystemPromptBuilder;
use crate::openhuman::providers::{ChatMessage, ChatRequest, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolSpec};
use std::collections::HashSet;
use std::path::PathBuf;
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
    let archetype_prompt_body = load_prompt_source(
        &definition.system_prompt,
        &parent.workspace_dir,
    )?;

    // ── Resolve model + temperature ────────────────────────────────────
    let model = definition.model.resolve(&parent.model_name);
    let temperature = definition.temperature;

    // ── Filter tools per definition + per-spawn override ───────────────
    let allowed_indices = filter_tool_indices(
        &parent.all_tools,
        &definition.tools,
        &definition.disallowed_tools,
        options
            .skill_filter_override
            .as_deref()
            .or(definition.skill_filter.as_deref()),
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
    let prompt_builder = SystemPromptBuilder::for_subagent(
        archetype_prompt_body,
        definition.omit_identity,
        definition.omit_safety_preamble,
        definition.omit_skills_catalog,
    );
    // We pass an empty tools slice and identity_config=None so the prompt
    // builder can produce a reproducible string. The ToolsSection still
    // renders our filtered tools because we pass them through PromptContext.
    let tools_for_prompt: Vec<&Box<dyn Tool>> =
        allowed_indices.iter().map(|&i| &parent.all_tools[i]).collect();
    // PromptContext expects a slice of Box<dyn Tool>; we can't construct
    // one from a slice of references without cloning. Build a thin
    // wrapper Vec by cloning Arcs would be ideal — but Box isn't Clone.
    // Workaround: render the prompt manually here using a tiny inline
    // composer that mirrors what SystemPromptBuilder would produce.
    let _ = tools_for_prompt;

    let system_prompt = render_subagent_system_prompt(
        &prompt_builder,
        &parent.workspace_dir,
        &model,
        &allowed_indices,
        &parent.all_tools,
        definition,
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
        let assistant_history_content = build_native_assistant_payload(&response_text, &native_calls);
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
    serde_json::from_str(arguments).unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool filtering
// ─────────────────────────────────────────────────────────────────────────────

/// Returns indices into `parent_tools` for the tools the sub-agent may
/// invoke. Index-based filtering avoids cloning `Box<dyn Tool>` (which
/// isn't Clone) and lets us reuse the parent's existing instances.
fn filter_tool_indices(
    parent_tools: &[Box<dyn Tool>],
    scope: &ToolScope,
    disallowed: &[String],
    skill_filter: Option<&str>,
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
    workspace_dir: &PathBuf,
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
/// The rendered prompt mirrors the layout of `for_subagent` but is
/// composed inline so the sub-agent's tool catalogue contains only the
/// filtered tools.
fn render_subagent_system_prompt(
    _builder: &SystemPromptBuilder,
    workspace_dir: &PathBuf,
    model_name: &str,
    allowed_indices: &[usize],
    parent_tools: &[Box<dyn Tool>],
    definition: &AgentDefinition,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // 1. Archetype role prompt (already loaded into definition source).
    if let PromptSource::Inline(body) = &definition.system_prompt {
        if !body.trim().is_empty() {
            out.push_str(body.trim_end());
            out.push_str("\n\n");
        }
    }
    // (For File sources we load above and write into the prompt via the
    // builder's archetype section. To keep this rendering self-contained
    // and avoid double-loading, we don't repeat that here — the file
    // body is injected by the caller before invoking the runner via the
    // SystemPromptBuilder for_subagent() variant.)

    // 2. Filtered tool catalogue.
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
    out.push_str("Use the provided tools to accomplish the task. Reply with a concise, dense \
                 final answer when you have one — the parent agent will weave it back into the \
                 user-visible response.\n\n");

    // 4. Workspace + datetime so the model knows where it is and when.
    let _ = writeln!(out, "## Workspace\n\nWorking directory: `{}`\n", workspace_dir.display());
    let now = chrono::Local::now();
    let _ = writeln!(
        out,
        "## Current Date & Time\n\n{} ({})\n",
        now.format("%Y-%m-%d %H:%M:%S"),
        now.format("%Z")
    );

    // 5. Runtime banner — model name only (no host info; sub-agents
    //    don't need that).
    let _ = writeln!(out, "## Runtime\n\nModel: {model_name}");

    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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

    use async_trait::async_trait;
    use crate::openhuman::tools::{PermissionLevel, ToolResult};

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
        let parent: Vec<Box<dyn Tool>> = vec![
            stub("alpha"),
            stub("beta"),
            stub("gamma"),
        ];
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
        assert_eq!(SubagentMode::Fork.as_str(), "fork");
    }
}
