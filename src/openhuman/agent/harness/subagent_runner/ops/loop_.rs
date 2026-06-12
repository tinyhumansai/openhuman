//! Sub-agent inner tool-call loop.
//!
//! Drives the iterative cycle of provider calls and tool execution until the
//! model returns without further tool calls (or the iteration budget is
//! exhausted). Unlike the main agent loop, this is isolated and returns only
//! the final text to be synthesised by the parent.

use std::collections::HashSet;

use crate::openhuman::agent::harness::fork_context::ParentExecutionContext;
use crate::openhuman::agent::harness::subagent_runner::handoff::ResultHandoffCache;
use crate::openhuman::agent::harness::subagent_runner::types::SubagentRunError;
use crate::openhuman::inference::provider::Provider;
use crate::openhuman::tools::{Tool, ToolSpec};

use super::super::tool_prep::build_text_mode_tool_instructions;
use super::checkpoint::SubagentCheckpoint;
use super::observer::SubagentObserver;
use super::provider::LazyToolkitResolver;
use super::tool_source::SubagentToolSource;

/// Cumulative usage stats gathered across all provider calls in the loop.
#[derive(Debug, Clone, Default)]
pub(super) struct AggregatedUsage {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) charged_amount_usd: f64,
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
pub(super) async fn run_inner_loop(
    provider: &dyn Provider,
    history: &mut Vec<crate::openhuman::inference::provider::ChatMessage>,
    parent_tools: &[Box<dyn Tool>],
    extra_tools: Vec<Box<dyn Tool>>,
    tool_specs: &[ToolSpec],
    allowed_names: HashSet<String>,
    lazy_resolver: Option<LazyToolkitResolver>,
    model: &str,
    // User-configured vision flag for `model` (computed at the call site), set as
    // the `current_model_vision` task-local around the engine call so a flagged
    // custom/BYOK sub-agent model can forward images.
    model_vision: bool,
    temperature: f64,
    max_iterations: usize,
    task_id: &str,
    agent_id: &str,
    worker_thread_id: Option<String>,
    handoff_cache: Option<&ResultHandoffCache>,
    parent: &ParentExecutionContext,
    extended_policy: bool,
) -> Result<(String, usize, AggregatedUsage, Option<String>), SubagentRunError> {
    // An autonomous skill run (set via `with_autonomous_iter_cap`) lifts the
    // per-agent cap so sub-agents run until done / the circuit breaker trips.
    let max_iterations = super::super::autonomous::autonomous_iter_cap()
        .map(|cap| cap.max(max_iterations))
        .unwrap_or(max_iterations)
        .max(1);

    // Sub-agent transcript stem — computed once up front so every iteration's
    // persist resolves to the same file: `{parent_chain}__{unix_ts}_{agent_id}`.
    let child_session_key = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let unix_ts = now.as_secs();
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

    // ── Text-mode override for integrations_agent ──
    // Large Composio toolkits compile into provider grammars that blow the
    // 65 535-rule ceiling, so for `integrations_agent` we omit `tools: [...]`
    // and describe them in the system prompt as prose, parsing `<tool_call>`
    // tags out of the model's response. Forcing `request_specs() == &[]` makes
    // the engine skip native tools and fall back to its XML parse + batched
    // `[Tool results]` path — exactly what text mode needs.
    let force_text_mode = agent_id == "integrations_agent" && !tool_specs.is_empty();
    if force_text_mode {
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

    let advertised_specs: Vec<ToolSpec> = if force_text_mode {
        Vec::new()
    } else {
        tool_specs.to_vec()
    };

    let mut tool_source = SubagentToolSource {
        parent_tools,
        extra_tools,
        allowed_names,
        lazy_resolver,
        advertised_specs,
        handoff_cache,
        policy: crate::openhuman::tools::policy::DefaultToolPolicy,
        agent_id: agent_id.to_string(),
    };
    let mut observer = SubagentObserver {
        worker_thread_id,
        workspace_dir: parent.workspace_dir.clone(),
        transcript_stem,
        agent_id: agent_id.to_string(),
        task_id: task_id.to_string(),
        force_text_mode,
        usage: AggregatedUsage::default(),
    };
    let checkpoint = SubagentCheckpoint {
        provider,
        model: model.to_string(),
        temperature,
        agent_id: agent_id.to_string(),
    };
    let progress = super::super::super::engine::SubagentProgress {
        sink: parent.on_progress.clone(),
        agent_id: agent_id.to_string(),
        task_id: task_id.to_string(),
        extended_policy,
    };

    let parser = super::super::super::engine::DefaultParser;
    // Heap-allocate the child `run_turn_engine` state machine. Sub-agents
    // run as nested polls inside the *parent* agent's `run_turn_engine`
    // (the orchestrator → tool exec → `dispatch_subagent` → `run_subagent`
    // chain), so without the box the parent's tokio worker poll stack
    // also has to carry the child engine's ~600-line generator. That
    // crosses the 2 MiB tokio worker default and aborts with
    // "thread 'tokio-rt-worker' has overflowed its stack" — see the
    // `chat-harness-subagent` Playwright lane crash logged here:
    // `[subagent_runner] dispatching agent_id=researcher ... → fatal
    // runtime error: stack overflow`. Boxing here breaks the stack
    // accumulation at the recursion boundary. Smoke-tested in
    // `nested_subagent_dispatch_runs_on_a_constrained_worker_stack`;
    // the deep end-to-end catcher is the `chat-harness-subagent`
    // Playwright spec.
    let outcome = super::super::super::model_vision_context::with_current_model_vision(
        model_vision,
        Box::pin(super::super::super::engine::run_turn_engine(
            provider,
            history,
            &mut tool_source,
            &progress,
            &mut observer,
            &checkpoint,
            &parser,
            "subagent",
            model,
            temperature,
            true, // silent — sub-agents never echo to stdout
            &crate::openhuman::config::MultimodalConfig::default(),
            &crate::openhuman::config::MultimodalFileConfig::default(),
            max_iterations,
            None, // sub-agents don't stream a draft
            &["ask_user_clarification"],
            None, // sub-agents don't support run-queue steering
        )),
    )
    .await?;

    Ok((
        outcome.text,
        outcome.iterations as usize,
        observer.usage,
        outcome.early_exit_tool,
    ))
}
