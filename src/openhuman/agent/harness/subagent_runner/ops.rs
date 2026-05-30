//! Sub-agent execution entry points and the inner tool-call loop.
//!
//! The public runner lives in [`run_subagent`]. It dispatches to
//! [`run_typed_mode`] (narrow prompt + filtered tools) which builds a
//! brand-new system prompt and a filtered tool list for the requested
//! archetype, then drives provider calls and tool execution until the
//! model returns without further tool calls (or the iteration budget
//! is exhausted).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use super::super::fork_context::{current_parent, ParentExecutionContext};
use super::super::session::transcript;
use super::extract_tool::ExtractFromResultTool;
use super::handoff::{
    build_handoff_placeholder, clean_tool_output, ResultHandoffCache,
    HANDOFF_OVERSIZE_THRESHOLD_TOKENS,
};
use super::tool_prep::{
    build_text_mode_tool_instructions, filter_tool_indices, is_subagent_spawn_tool,
    load_prompt_source, top_k_for_toolkit,
};
use super::types::{SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome};
use crate::openhuman::agent::harness::definition::{AgentDefinition, PromptSource};
use crate::openhuman::agent::harness::{
    current_spawn_depth, with_current_sandbox_mode, with_spawn_depth, MAX_SPAWN_DEPTH,
};
use crate::openhuman::context::prompt::{
    render_subagent_system_prompt, PromptContext, PromptTool, SubagentRenderOptions,
};
use crate::openhuman::inference::provider::{ChatMessage, ChatRequest, Provider};
use crate::openhuman::memory_conversations::ConversationMessage;
use crate::openhuman::tools::{Tool, ToolCategory, ToolSpec};

/// Prompt suffix injected into every typed sub-agent run.
///
/// Purpose:
/// - make the child explicitly aware it is acting as a sub-agent
/// - keep delegated outputs concise so parent-context growth stays bounded
/// - discourage verbose restatement of the delegated task/context
const SUBAGENT_ROLE_CONTRACT_SUFFIX: &str = "## Sub-agent Role Contract\n\n\
You are a sub-agent working for a parent OpenHuman agent, not a direct end-user assistant.\n\
- Stay tightly scoped to the delegated task.\n\
- Keep tool arguments and follow-up prompts compact, include only required fields/context.\n\
- Keep your final response concise and synthesis-ready for the parent, prefer short bullets or short paragraphs.\n\
- Do not restate the full task/context unless strictly required for correctness.\n";

fn append_subagent_role_contract(base_prompt: String, agent_id: &str) -> String {
    if base_prompt.contains(SUBAGENT_ROLE_CONTRACT_SUFFIX.trim()) {
        tracing::debug!(
            agent_id = %agent_id,
            base_chars = base_prompt.chars().count(),
            "[subagent_runner] sub-agent role contract already present in system prompt"
        );
        return base_prompt;
    }

    let mut prompt = base_prompt;
    if !prompt.ends_with('\n') {
        prompt.push('\n');
    }
    prompt.push('\n');
    prompt.push_str(SUBAGENT_ROLE_CONTRACT_SUFFIX);

    tracing::debug!(
        agent_id = %agent_id,
        suffix_chars = SUBAGENT_ROLE_CONTRACT_SUFFIX.chars().count(),
        final_chars = prompt.chars().count(),
        "[subagent_runner] appended sub-agent role contract to system prompt"
    );

    prompt
}

/// Resolve a sub-agent's `(provider, model)` based on its declarative
/// `[model]` spec.
///
///   - inline `model` override — highest precedence for one call.
///   - config-level pin — `[orchestrator] model` or `[teams.*]`
///     `lead_model` / `agent_model`, when present.
///   - `Inherit` — use the parent's provider AND model. Literally
///     "do what the parent does".
///   - `Hint(workload)` — build a fresh provider via the per-workload
///     factory (e.g. `integrations_agent`'s `[model] hint = "agentic"`
///     resolves to whatever `agentic_provider` is routed to in
///     AI Settings). The factory returns the *exact* model id for that
///     workload — the OpenHuman backend and every third-party provider
///     accept exact model names, so there's no `{hint}-v1` synthesis
///     anywhere on this path.
///   - `Exact(name)` — escape hatch: use the parent's provider with
///     this model name overriding the parent's. Callers are expected
///     to know the model is valid for the parent's provider; the enum
///     is the wrong place to encode provider switching, which belongs
///     to `Hint` + AI-settings routing.
///
/// `config` is `None` when the live `Config::load_or_init()` failed
/// (rare — transient I/O). Both `None` config and factory build errors
/// fall back to `(parent_provider, parent_model)` so a config glitch
/// can't sink sub-agent execution entirely.
///
/// The async part (config load) is hoisted out of the caller so this
/// helper stays sync and can be exercised by a focused unit test
/// without spinning up a `tokio::test` runtime per case.
pub(super) fn resolve_subagent_provider(
    spec: &crate::openhuman::agent::harness::definition::ModelSpec,
    agent_id: &str,
    config: Option<&crate::openhuman::config::Config>,
    parent_provider: std::sync::Arc<dyn Provider>,
    parent_model: String,
    is_team_lead: bool,
    model_override: Option<&str>,
) -> (std::sync::Arc<dyn Provider>, String) {
    use crate::openhuman::agent::harness::definition::ModelSpec;
    if let Some(model) = model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        log::debug!(
            "[subagent_runner] agent_id={} using inline model override model={}",
            agent_id,
            model
        );
        return (parent_provider, model.to_string());
    }

    if let Some(model) = config.and_then(|cfg| cfg.configured_agent_model(agent_id, is_team_lead)) {
        log::debug!(
            "[subagent_runner] agent_id={} using config-level model pin model={}",
            agent_id,
            model
        );
        return (parent_provider, model.to_string());
    }

    match spec {
        ModelSpec::Hint(workload) => match config {
            Some(cfg) => {
                match crate::openhuman::inference::provider::create_chat_provider(workload, cfg) {
                    Ok((p, m)) => {
                        log::info!(
                        "[subagent_runner] role={} agent_id={} resolved via workload factory model={}",
                        workload, agent_id, m
                    );
                        (std::sync::Arc::from(p), m)
                    }
                    Err(e) => {
                        let suggested_key = match workload.as_str() {
                            "summarization" | "memory" => "memory_provider".to_string(),
                            _ => format!("{workload}_provider"),
                        };
                        log::warn!(
                            "[subagent_runner] workload='{}' provider build failed for agent_id={} error='{}' \
                             falling back to parent provider (parent_model='{}'). \
                             Consider setting {} in config.",
                            workload,
                            agent_id,
                            e,
                            parent_model,
                            suggested_key
                        );
                        (parent_provider, parent_model)
                    }
                }
            }
            None => {
                log::warn!(
                    "[subagent_runner] config load failed for workload '{}' (agent_id={}) — \
                     falling back to parent provider + parent model '{}'",
                    workload,
                    agent_id,
                    parent_model
                );
                (parent_provider, parent_model)
            }
        },
        ModelSpec::Inherit => (parent_provider, parent_model),
        ModelSpec::Exact(name) => (parent_provider, name.clone()),
    }
}

/// Lazy resolver that lets `integrations_agent` recover when the model
/// calls a Composio action slug that exists in the bound toolkit's full
/// catalogue but was filtered out of the up-front fuzzy top-K. On a
/// match we build the [`ComposioActionTool`] on demand so the call
/// dispatches normally instead of dead-ending in
/// `Error: tool '...' is not available`.
///
/// Holds an [`Arc<Config>`] rather than a pre-baked
/// [`crate::openhuman::composio::ComposioClient`] so the live
/// `composio.mode` toggle is honoured per execute — see
/// [`crate::openhuman::composio::ComposioActionTool`] and issue #1710.
struct LazyToolkitResolver {
    config: std::sync::Arc<crate::openhuman::config::Config>,
    actions: Vec<crate::openhuman::context::prompt::ConnectedIntegrationTool>,
}

impl LazyToolkitResolver {
    fn resolve(&self, name: &str) -> Option<Box<dyn Tool>> {
        let action = self.find_action(name)?;
        Some(Box::new(
            crate::openhuman::composio::ComposioActionTool::new(
                self.config.clone(),
                action.name.clone(),
                action.description.clone(),
                action.parameters.clone(),
            ),
        ))
    }

    /// Match a model-supplied tool name to a real toolkit action, tolerant
    /// of the near-miss slugs models routinely emit — case differences and
    /// separator/prefix drift (bug-report-2026-05-26 A2). Tries, in order:
    /// exact, case-insensitive, then a normalized alphanumeric match
    /// (accepted only when **unique**, so a fabricated slug can't silently
    /// resolve to the wrong action — those still fall through to the
    /// "tool not available" error, which lists `known_slugs` for the model
    /// to self-correct).
    fn find_action(
        &self,
        name: &str,
    ) -> Option<&crate::openhuman::context::prompt::ConnectedIntegrationTool> {
        if let Some(action) = self.actions.iter().find(|a| a.name == name) {
            return Some(action);
        }
        if let Some(action) = self
            .actions
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        {
            tracing::debug!(
                requested = %name,
                matched = %action.name,
                "[subagent_runner] resolved tool by case-insensitive match"
            );
            return Some(action);
        }
        let norm = normalize_slug(name);
        if !norm.is_empty() {
            let mut matches = self
                .actions
                .iter()
                .filter(|a| normalize_slug(&a.name) == norm);
            if let Some(action) = matches.next() {
                if matches.next().is_none() {
                    tracing::info!(
                        requested = %name,
                        matched = %action.name,
                        "[subagent_runner] resolved tool by normalized-slug match"
                    );
                    return Some(action);
                }
                // Ambiguous: 2+ actions normalize to the same slug (e.g.
                // `read_file` and `ReadFile` → `readfile`). We deliberately
                // refuse to guess. Warn (not debug): a slug collision is a
                // toolkit configuration anomaly that should surface in normal
                // operator logs, not stay hidden behind debug filtering.
                tracing::warn!(
                    requested = %name,
                    norm = %norm,
                    "[subagent_runner] ambiguous normalized-slug match — multiple actions resolve to the same slug; not resolving"
                );
            }
        }
        None
    }

    /// Slugs from the bound toolkit, for inclusion in unknown-tool
    /// errors so the model can self-correct without burning a turn.
    fn known_slugs(&self) -> Vec<&str> {
        self.actions.iter().map(|a| a.name.as_str()).collect()
    }
}

/// Lowercased, non-alphanumerics stripped — collapses separator/prefix
/// drift (`GOOGLESLIDES_BATCH_UPDATE` vs `googleslides_batch_update`) so
/// near-miss tool slugs still resolve, while genuinely different slugs
/// (e.g. a hallucinated `GMAIL_GET_LAST_3_MESSAGES`) stay distinct.
fn normalize_slug(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

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
    // task-local for every tool invocation inside this run. Tools that
    // want to gate on it (e.g. `composio_execute` rejecting
    // Write/Admin slugs under `ReadOnly`) read it via
    // `current_sandbox_mode()`; tools that don't care just ignore it.
    // Box-pin the inner future so the large `run_typed_mode` state machine
    // lives on the heap. Two stacked `task_local::scope` wrappers
    // (`with_spawn_depth` + `with_current_sandbox_mode`) plus the deeply
    // nested provider/tool loop inside `run_typed_mode` are otherwise large
    // enough — under `cargo-llvm-cov` instrumentation in particular — to
    // overflow tokio's 2 MiB per-thread test stack. See #2234 CI failure.
    let mut outcome = with_spawn_depth(attempted_depth, async {
        with_current_sandbox_mode(definition.sandbox_mode, async {
            Box::pin(run_typed_mode(
                definition,
                task_prompt,
                &options,
                &parent,
                &task_id,
            ))
            .await
        })
        .await
    })
    .await?;

    // Truncate result to the definition's cap if set.
    // Use char-count (not byte-length) to avoid panicking on multi-byte
    // UTF-8 sequences at the truncation boundary.
    if let Some(cap) = definition.max_result_chars {
        let original_chars = outcome.output.chars().count();
        if original_chars > cap {
            tracing::debug!(
                agent_id = %definition.id,
                original_chars,
                cap,
                "[subagent_runner] truncating oversized result to max_result_chars cap"
            );
            // Find the byte offset of the cap-th character boundary so
            // `truncate` never lands mid-codepoint.
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
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed mode — narrow prompt, filtered tools, cheaper model
// ─────────────────────────────────────────────────────────────────────────────

/// Deduplicate assembled tool specs by name, keeping the first occurrence.
///
/// The sub-agent's `filtered_specs` is a `Vec` assembled from
/// `parent.all_tool_specs` indices plus dynamic tools, so a delegation tool can
/// shadow a same-named skill/integration tool (common for the wide-set
/// `tools_agent`), leaving two specs with the same name. Strict providers reject
/// such a request with `400 "Tool names must be unique."` The main-agent path
/// dedups via [`session::builder::dedup_visible_tool_specs`]; this separate
/// sub-agent assembly must do the same.
///
/// First occurrence wins so registration-order semantics are preserved (tool
/// dispatch still resolves by name). Dropped duplicates are logged at `debug`
/// (diagnostic instrumentation, per the repo Rust logging guideline).
///
/// Extracted as a free function so the regression suite can exercise the dedup
/// without standing up the full `run_typed_mode` plumbing.
fn dedup_tool_specs_by_name(agent_id: &str, specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let mut seen: HashSet<String> = HashSet::with_capacity(specs.len());
    let mut deduped: Vec<ToolSpec> = Vec::with_capacity(specs.len());
    let mut dropped: Vec<String> = Vec::new();
    for spec in specs {
        if seen.insert(spec.name.clone()) {
            deduped.push(spec);
        } else {
            dropped.push(spec.name);
        }
    }
    if !dropped.is_empty() {
        tracing::debug!(
            agent_id = %agent_id,
            "[subagent_runner] dropped {} duplicate tool spec(s) before sending to provider: {:?}",
            dropped.len(),
            dropped
        );
    }
    deduped
}

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

    // Archetype prompt loading is deferred until AFTER tool filtering so
    // dynamic builders receive the final, filtered tool list (rather
    // than the parent's full registry). The actual
    // `load_prompt_source(...)` call lives just above
    // `render_subagent_system_prompt` below.

    // ── Refresh connected-integrations at spawn time ───────────────────
    //
    // The parent session's `connected_integrations` Vec is frozen at
    // session-start (see `session/turn.rs::fetch_connected_integrations`,
    // which only runs while `history.is_empty()` to preserve the
    // KV-cache prefix). That means a toolkit the user authorised mid-
    // thread — e.g. Calendly — is missing from `parent.connected_integrations`,
    // and the spawn-time toolkit lookup further down rejects it as
    // "not allowlisted / not connected" until the user starts a new
    // thread or restarts the app.
    //
    // Re-fetch from the global integrations cache here. The cache is
    // invalidated by `ComposioConnectionCreatedSubscriber` once the
    // OAuth handshake reaches ACTIVE/CONNECTED, so this call returns
    // the fresh list almost for free on the warm path. Fall back to
    // the parent's frozen list when the live fetch returns empty (no
    // signed-in user, backend unreachable, …) so offline / not-signed-
    // in behaviour is unchanged.
    let live_integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration> = {
        // Mode-aware "is the user able to call composio at all?" probe.
        // `create_composio_client` returns `Ok(_)` whenever the user has
        // EITHER a backend session token (backend mode) OR a stored
        // direct-mode API key — so a direct-mode user with only a key
        // in the keychain is now correctly recognised as "signed in"
        // for the spawn-time refresh path (#1710 Wave 2). Pre-fix this
        // gate read `parent.composio_client.is_none()`, which was only
        // ever populated in backend mode and silently skipped the live
        // refresh for direct-mode users.
        //
        // We resolve here purely as a probe — the client itself is
        // dropped immediately. Per-action dispatch below (and inside
        // `ComposioActionTool::execute`) re-resolves through the
        // factory so the live `composio.mode` toggle keeps winning.
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
                    // `fetch_connected_integrations_status` distinguishes
                    // an authoritative empty list (user disconnected
                    // their last integration mid-thread) from
                    // backend-unavailable (no client / transient error).
                    // Adopt the authoritative case as truth — even when
                    // empty — so a revoked toolkit really disappears
                    // from the spawn pre-flight; only fall back to the
                    // parent's frozen list when the backend explicitly
                    // can't answer.
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
                    // Real failure — config couldn't be read, so the
                    // backend client can't be built either. Use the
                    // parent's frozen list as a best-effort fallback so
                    // the spawn can still proceed for sessions that
                    // were established when config was healthy.
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
    let mut lazy_resolver: Option<LazyToolkitResolver> = None;
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

        if let Some(tk) = toolkit_filter {
            // Load a fresh `Arc<Config>` for the dynamic
            // `ComposioActionTool`s registered below. Pre-Wave-2 this
            // path was gated on `parent.composio_client.as_ref()` —
            // backend-only by construction, so direct-mode users were
            // silently dropped here even after they'd connected the
            // toolkit on `app.composio.dev`. Resolving the client
            // through the mode-aware factory closes that gap and keeps
            // the registration in lockstep with `ComposioActionTool`'s
            // per-call dispatch (#1710).
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

            // Resolve the live client kind for the catalogue refresh
            // path. Backend mode keeps the existing
            // `fetch_toolkit_actions` round-trip. Direct mode mirrors
            // the `ComposioListToolsTool` short-circuit — the backend
            // toolkit allowlist isn't authoritative for a personal
            // Composio tenant, so we fall back to the parent's cached
            // catalogue rather than emit a misleading "couldn't fetch"
            // surface (#1710 Wave 2).
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

            // The spawn_subagent pre-flight already verified the
            // toolkit is in the allowlist AND has an active
            // connection, so the matching entry must be present and
            // marked connected. Defensive lookup anyway. Reads from
            // `live_integrations` (refreshed above) rather than the
            // session-frozen `parent.connected_integrations` so a
            // mid-thread `composio_authorize` is visible without a
            // new thread / restart.
            if let Some(cached_integration) = live_integrations
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
                        // Direct mode has no backend-allowlist catalogue
                        // refresh path — the personal Composio tenant
                        // governs availability. Mirror the
                        // `ComposioListToolsTool` direct-mode short-
                        // circuit and fall back to the cached catalogue
                        // bulk-fetched at session start (#1710 Wave 2).
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
                    // Inherit the cached gated set: this spawn path only
                    // refreshes the *visible* (callable) actions from the
                    // backend; the gated/unlock-hint surface is computed
                    // by `fetch_connected_integrations_uncached` against
                    // the user pref and doesn't change per-spawn.
                    gated_tools: cached_integration.gated_tools.clone(),
                    connected: cached_integration.connected,
                    // Inherit the cached non-active status — this spawn
                    // path only fires on connected toolkits, but keep the
                    // field consistent with the source row for #2365.
                    non_active_status: cached_integration.non_active_status.clone(),
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
                // Stash the full catalogue so the inner loop can lazily
                // register actions that the fuzzy top-K dropped — the
                // model often picks the right slug anyway and the
                // existing fuzzy filter exists only to keep schemas out
                // of the system prompt, not to gate execution.
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

    // Build provider-visible tool schemas in EXECUTION-PRECEDENCE order:
    // `dynamic_tools` (extra_tools at runtime) before parent specs, because
    // the inner loop's name lookup (see end of this fn) resolves
    // `extra_tools` first and only falls back to `parent_tools`. Aligning
    // the dedup order with the runtime lookup order guarantees the schema
    // the model sees and the tool that actually executes describe the same
    // behaviour. (CodeRabbit review on PR #2446.)
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
    // Dedup by name: first occurrence wins. Dynamic Composio action tools
    // can share a name with an inherited parent-registry spec when the
    // agent's AllowedAll scope includes a same-named skill tool. Some
    // providers (Anthropic, OpenHuman cloud after the uniqueness-enforcement
    // rollout) 400 on duplicate tool names — see TAURI-RUST-4. Because
    // `filtered_specs` is in execution order (dynamic first), the kept
    // schema matches what the runtime will actually dispatch.
    let filtered_specs =
        crate::openhuman::agent::harness::session::dedup_visible_tool_specs(filtered_specs);

    // Dedup by tool name before the specs reach the provider (see
    // `dedup_tool_specs_by_name` for why duplicates appear here).
    let filtered_specs = dedup_tool_specs_by_name(&definition.id, filtered_specs);

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
        curated_snapshot: None,
        user_identity: crate::openhuman::app_state::peek_cached_current_user_identity(),
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        workflows: &[],
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

    let system_prompt = append_subagent_role_contract(system_prompt, &definition.id);

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
        if let Some(ref mem_ctx) = *parent.memory_context {
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
        subagent_provider.as_ref(),
        &mut history,
        &parent.all_tools,
        dynamic_tools,
        &filtered_specs,
        allowed_names,
        lazy_resolver,
        &model,
        temperature,
        definition.max_iterations,
        task_id,
        &definition.id,
        options.worker_thread_id.clone(),
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
    extra_tools: Vec<Box<dyn Tool>>,
    tool_specs: &[ToolSpec],
    allowed_names: HashSet<String>,
    lazy_resolver: Option<LazyToolkitResolver>,
    model: &str,
    temperature: f64,
    max_iterations: usize,
    task_id: &str,
    agent_id: &str,
    worker_thread_id: Option<String>,
    handoff_cache: Option<&ResultHandoffCache>,
    parent: &ParentExecutionContext,
) -> Result<(String, usize, AggregatedUsage), SubagentRunError> {
    // An autonomous skill run (set via `with_autonomous_iter_cap`) lifts the
    // per-agent cap so sub-agents run until done / the circuit breaker trips.
    let max_iterations = super::autonomous::autonomous_iter_cap()
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
    let progress = super::super::engine::SubagentProgress {
        sink: parent.on_progress.clone(),
        agent_id: agent_id.to_string(),
        task_id: task_id.to_string(),
    };

    let parser = super::super::engine::DefaultParser;
    let outcome = super::super::engine::run_turn_engine(
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
        max_iterations,
        None, // sub-agents don't stream a draft
    )
    .await?;

    Ok((outcome.text, outcome.iterations as usize, observer.usage))
}

/// Apply the progressive-disclosure handoff to a tool result. If a cache is
/// present and the (cleaned) result is large and not an error / not from the
/// extractor tool, stash the raw payload and substitute a short placeholder the
/// sub-agent can drill into with `extract_from_result`. Errors and
/// already-extracted output pass through unchanged.
fn apply_handoff(
    cache: &ResultHandoffCache,
    tool_name: &str,
    task_id: &str,
    agent_id: &str,
    result_text: String,
) -> String {
    let skip_cleaning = tool_name == "extract_from_result" || result_text.starts_with("Error");
    let cleaned = if skip_cleaning {
        result_text
    } else {
        let pre_len = result_text.len();
        let cleaned = clean_tool_output(&result_text);
        if cleaned.len() < pre_len {
            tracing::debug!(
                tool = %tool_name,
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
        let id = cache.store(tool_name.to_string(), cleaned.clone());
        let placeholder = build_handoff_placeholder(tool_name, &id, &cleaned);
        tracing::info!(
            task_id = %task_id,
            agent_id = %agent_id,
            tool = %tool_name,
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
}

/// Sub-agent [`ToolSource`]: looks up tools in `extra_tools` then the parent
/// registry, lazily registers toolkit actions the fuzzy filter omitted, rejects
/// names outside the allowlist, and routes execution through the shared
/// [`run_one_tool`] (so sub-agents now get the same approval gate, audit,
/// credential scrub, tokenjuice and timeout as the channel loop), then applies
/// the progressive-disclosure handoff.
struct SubagentToolSource<'a> {
    parent_tools: &'a [Box<dyn Tool>],
    extra_tools: Vec<Box<dyn Tool>>,
    allowed_names: HashSet<String>,
    lazy_resolver: Option<LazyToolkitResolver>,
    advertised_specs: Vec<ToolSpec>,
    handoff_cache: Option<&'a ResultHandoffCache>,
    policy: crate::openhuman::tools::policy::DefaultToolPolicy,
    agent_id: String,
}

#[async_trait::async_trait]
impl super::super::engine::ToolSource for SubagentToolSource<'_> {
    fn request_specs(&self) -> &[ToolSpec] {
        &self.advertised_specs
    }

    async fn execute_call(
        &mut self,
        call: &super::super::parse::ParsedToolCall,
        iteration: usize,
        progress: &dyn super::super::engine::ProgressReporter,
        progress_call_id: &str,
    ) -> super::super::engine::ToolRunResult {
        // Lazy registration: a call for an unknown tool that matches a real
        // action slug in the bound toolkit gets built on the spot and admitted
        // to the allowlist. The fuzzy top-K filter keeps schemas out of the
        // prompt, not out of execution.
        if !self.allowed_names.contains(&call.name) {
            if let Some(resolver) = self.lazy_resolver.as_ref() {
                if let Some(tool) = resolver.resolve(&call.name) {
                    tracing::info!(
                        agent_id = %self.agent_id,
                        tool = %call.name,
                        "[subagent_runner] lazily registered toolkit action outside fuzzy top-K"
                    );
                    self.allowed_names.insert(tool.name().to_string());
                    self.extra_tools.push(tool);
                }
            }
        }

        if !self.allowed_names.contains(&call.name) {
            tracing::warn!(
                agent_id = %self.agent_id,
                tool = %call.name,
                "[subagent_runner] tool not in allowlist for this sub-agent"
            );
            let iteration_u32 = (iteration + 1) as u32;
            progress
                .tool_started(progress_call_id, &call.name, &call.arguments, iteration_u32)
                .await;
            let mut available: Vec<&str> = self.allowed_names.iter().map(|s| s.as_str()).collect();
            if let Some(resolver) = self.lazy_resolver.as_ref() {
                available.extend(resolver.known_slugs());
            }
            available.sort_unstable();
            available.dedup();
            let text = format!(
                "Error: tool '{}' is not available to the {} sub-agent. Available tools: {}",
                call.name,
                self.agent_id,
                available.join(", ")
            );
            progress
                .tool_completed(
                    progress_call_id,
                    &call.name,
                    false,
                    text.chars().count(),
                    0,
                    iteration_u32,
                )
                .await;
            return super::super::engine::ToolRunResult {
                text,
                success: false,
            };
        }

        let tool_opt: Option<&dyn Tool> = self
            .extra_tools
            .iter()
            .find(|t| t.name() == call.name)
            .or_else(|| self.parent_tools.iter().find(|t| t.name() == call.name))
            .map(|b| b.as_ref());
        let outcome = super::super::engine::run_one_tool(
            tool_opt,
            call,
            iteration,
            progress,
            &self.policy,
            None,
            progress_call_id,
        )
        .await;

        let text = match self.handoff_cache {
            Some(cache) => apply_handoff(cache, &call.name, "", &self.agent_id, outcome.text),
            None => outcome.text,
        };
        super::super::engine::ToolRunResult {
            text,
            success: outcome.success,
        }
    }
}

/// Sub-agent [`TurnObserver`]: accumulates usage, persists the per-iteration
/// transcript, and mirrors assistant intents / tool results / final responses
/// to the spawn's worker thread (when one is attached).
struct SubagentObserver {
    worker_thread_id: Option<String>,
    workspace_dir: std::path::PathBuf,
    transcript_stem: String,
    agent_id: String,
    task_id: String,
    force_text_mode: bool,
    usage: AggregatedUsage,
}

impl SubagentObserver {
    fn append_worker_message(
        &self,
        content: String,
        sender: String,
        extra_metadata: serde_json::Value,
    ) {
        let Some(ref thread_id) = self.worker_thread_id else {
            return;
        };
        let message = ConversationMessage {
            id: format!("{}:{}", sender, uuid::Uuid::new_v4()),
            content,
            message_type: "text".to_string(),
            extra_metadata,
            sender,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(err) = crate::openhuman::memory_conversations::append_message(
            self.workspace_dir.clone(),
            thread_id,
            message,
        ) {
            tracing::debug!(
                agent_id = %self.agent_id,
                thread_id = %thread_id,
                error = %err,
                "[subagent_runner] failed to append message to worker thread"
            );
        }
    }

    fn persist_transcript(&self, history: &[ChatMessage]) {
        let path = match transcript::resolve_keyed_transcript_path(
            &self.workspace_dir,
            &self.transcript_stem,
        ) {
            Ok(p) => p,
            Err(err) => {
                tracing::debug!(
                    agent_id = %self.agent_id,
                    error = %err,
                    "[subagent_runner] failed to resolve transcript path"
                );
                return;
            }
        };
        let now = chrono::Utc::now().to_rfc3339();
        let meta = transcript::TranscriptMeta {
            agent_name: self.agent_id.clone(),
            dispatcher: "native".into(),
            created: now.clone(),
            updated: now,
            turn_count: 1,
            input_tokens: self.usage.input_tokens,
            output_tokens: self.usage.output_tokens,
            cached_input_tokens: self.usage.cached_input_tokens,
            charged_amount_usd: self.usage.charged_amount_usd,
            thread_id: crate::openhuman::inference::provider::thread_context::current_thread_id(),
        };
        if let Err(err) = transcript::write_transcript(&path, history, &meta, None) {
            tracing::debug!(
                agent_id = %self.agent_id,
                error = %err,
                "[subagent_runner] failed to write transcript"
            );
        }
    }
}

#[async_trait::async_trait]
impl super::super::engine::TurnObserver for SubagentObserver {
    fn record_usage(
        &mut self,
        _model: &str,
        usage: &crate::openhuman::inference::provider::UsageInfo,
    ) {
        self.usage.input_tokens += usage.input_tokens;
        self.usage.output_tokens += usage.output_tokens;
        self.usage.cached_input_tokens += usage.cached_input_tokens;
        self.usage.charged_amount_usd += usage.charged_amount_usd;
    }

    fn on_assistant(
        &mut self,
        _display_text: &str,
        response_text: &str,
        _reasoning_content: Option<&str>,
        _native_tool_calls: &[crate::openhuman::inference::provider::ToolCall],
        parsed_calls: &[super::super::parse::ParsedToolCall],
        iteration: usize,
        is_final: bool,
    ) {
        let tool_calls = parsed_calls.len();
        let extra = if is_final {
            serde_json::json!({
                "scope": "worker_thread",
                "agent_id": self.agent_id,
                "task_id": self.task_id,
                "iteration": iteration + 1,
                "final": true,
            })
        } else {
            serde_json::json!({
                "scope": "worker_thread",
                "agent_id": self.agent_id,
                "task_id": self.task_id,
                "iteration": iteration + 1,
                "tool_calls": tool_calls,
            })
        };
        self.append_worker_message(response_text.to_string(), "agent".to_string(), extra);
    }

    fn on_tool_result(
        &mut self,
        call_id: &str,
        tool_name: &str,
        result_text: &str,
        _success: bool,
        iteration: usize,
    ) {
        // Native mode mirrors each tool result individually; text mode batches
        // them in `on_results_batch` instead.
        if self.force_text_mode {
            return;
        }
        self.append_worker_message(
            result_text.to_string(),
            "user".to_string(),
            serde_json::json!({
                "scope": "worker_thread",
                "agent_id": self.agent_id,
                "task_id": self.task_id,
                "iteration": iteration + 1,
                "tool_call_id": call_id,
                "tool_name": tool_name,
            }),
        );
    }

    fn on_results_batch(&mut self, content: &str, iteration: usize) {
        self.append_worker_message(
            content.to_string(),
            "user".to_string(),
            serde_json::json!({
                "scope": "worker_thread",
                "agent_id": self.agent_id,
                "task_id": self.task_id,
                "iteration": iteration + 1,
                "mode": "text",
            }),
        );
    }

    fn after_iteration(&mut self, history: &[ChatMessage], _iteration: usize) {
        self.persist_transcript(history);
    }
}

/// Sub-agent [`CheckpointStrategy`]: when the iteration cap is hit, summarize
/// the run-so-far into a resumable checkpoint (so the delegating agent can
/// continue from partial progress) instead of erroring. Falls back to a
/// deterministic digest summary if the summarization call fails or returns no
/// prose.
struct SubagentCheckpoint<'a> {
    provider: &'a dyn Provider,
    model: String,
    temperature: f64,
    agent_id: String,
}

#[async_trait::async_trait]
impl super::super::engine::CheckpointStrategy for SubagentCheckpoint<'_> {
    async fn on_max_iter(
        &self,
        digest: &str,
        max_iterations: usize,
    ) -> anyhow::Result<super::super::engine::CheckpointOutcome> {
        let agent_id = &self.agent_id;
        let deterministic = format!(
            "I reached my tool-call limit ({max_iterations} steps) before finishing this task. \
             Progress so far (tool calls + results):\n{digest}\n\nThe task is incomplete — the above is \
             what I accomplished; continue from here."
        );
        let summary_input = vec![ChatMessage::user(format!(
            "You are sub-agent `{agent_id}` and reached your tool-call limit before finishing. Here are \
             the tool calls you made and their results — compile a brief progress checkpoint (what you \
             accomplished, what still remains) for the agent that delegated to you. Do not call tools.\n\n{digest}"
        ))];
        match self
            .provider
            .chat(
                ChatRequest {
                    messages: &summary_input,
                    tools: None,
                    stream: None,
                },
                &self.model,
                self.temperature,
            )
            .await
        {
            Ok(resp) => {
                let usage = resp.usage.clone();
                let raw = resp.text.unwrap_or_default();
                let (prose, _) = super::super::parse::parse_tool_calls(&raw);
                let text = if prose.trim().is_empty() {
                    deterministic
                } else {
                    prose
                };
                Ok(super::super::engine::CheckpointOutcome { text, usage })
            }
            Err(e) => {
                tracing::warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "[subagent_runner] checkpoint summary call failed — using deterministic fallback"
                );
                Ok(super::super::engine::CheckpointOutcome {
                    text: deterministic,
                    usage: None,
                })
            }
        }
    }
}

fn parse_tool_arguments(arguments: &str) -> serde_json::Value {
    serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
}

/// Probe whether the user can call Composio at all under the current
/// config. Returns `true` when the mode-aware factory can build EITHER
/// a backend-mode client (legacy JWT-driven path) OR a direct-mode
/// client (BYO Composio API key). The resolved client is dropped
/// immediately — this is purely a "signed-in vs not" check used by the
/// spawn-time refresh path. Per-action dispatch resolves a fresh client
/// elsewhere via [`create_composio_client`] so the live `composio.mode`
/// toggle keeps winning.
///
/// Extracted as a free function so the regression suite can exercise
/// the same probe the runner uses without spinning up the full
/// `run_typed_mode` plumbing.
pub(crate) fn user_is_signed_in_to_composio(config: &crate::openhuman::config::Config) -> bool {
    crate::openhuman::composio::client::create_composio_client(config).is_ok()
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ops_dedup_tests.rs"]
mod dedup_tests;

#[cfg(test)]
#[path = "ops_truncation_tests.rs"]
mod truncation_tests;
