//! Shared root [`ParentExecutionContext`] builder for controller-spawned
//! orchestration tasks (#3374 PR4, extracted from the #3375 workflow engine).
//!
//! The workflow-run engine ([`workflow_runs::engine`]) and the agent-team
//! runtime ([`agent_teams::runtime`]) both need to spawn real sub-agents from a
//! background task that has **no** enclosing agent turn on the stack, so there is
//! no live parent execution context for the spawn to inherit — a naive spawn
//! fails with `NoParentContext` (the TAURI-RUST-HMW regression, #4337).
//!
//! The fix (proven in `triage::escalation::dispatch_target_agent`) is to build a
//! *root* [`ParentExecutionContext`] from a config-built [`Agent`] and hand it,
//! explicitly, to the loop that spawns. Every `spawn_agent` call then names this
//! root, inheriting a real provider, tool registry, memory, and model — the same
//! construction path `agent_chat` uses.
//!
//! [`with_root_parent`] is the single blessed entry point that builds the root
//! and passes it to the surface's closure, so a surface can neither hand-roll the
//! parent nor forget to supply one. Every background orchestration surface goes
//! through it.
//!
//! This was originally inlined in the workflow engine; #3374 PR4 lifted it here
//! so each surface reuses the exact same construction (and the single
//! registry-initialisation defense) rather than carrying a second copy of the
//! ~20-field context literal that could drift.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::agent::harness::fork_context::ParentExecutionContext;
use crate::agent::OpenHumanSessionHost;
use crate::config::Config;

const LOG_TARGET: &str = "agent_orchestration::parent_context";

/// Build a root [`ParentExecutionContext`] from a config-built [`Agent`].
///
/// Mirrors `triage::escalation::dispatch_target_agent` — the proven path for
/// running sub-agents without an enclosing agent turn. The caller supplies the
/// identity fields that distinguish one orchestration surface from another:
///
/// - `agent_definition_id` — labels the root parent (e.g. `"workflow_engine"`,
///   `"agent_team_runtime"`); surfaced in spawn metadata / logs.
/// - `channel` — the logical channel the spawned work belongs to (e.g.
///   `"workflow"`, `"team"`).
/// - `session_prefix` — the `session_id` is `"{session_prefix}-{uuid}"`, keeping
///   each surface's sessions namespaced and greppable.
///
/// Every other field is inherited verbatim from the config-built agent, so the
/// spawned children behave exactly like a normal sub-agent dispatch.
pub(crate) async fn build_root_parent(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    agent_definition_id: &str,
    channel: &str,
    session_prefix: &str,
) -> Result<ParentExecutionContext> {
    // Sub-agent spawns resolve their definition through the global
    // agent-definition registry, so it MUST be initialised before any spawn.
    // The full runtime boot (`bootstrap_core_runtime`) does this, but these
    // engines can also be reached from contexts that only built the HTTP router
    // (e.g. the JSON-RPC e2e harness) — so init defensively here. `OnceLock`
    // makes this idempotent: a no-op when the registry is already loaded.
    if crate::agent::harness::host_definition::AgentDefinitionRegistry::global().is_none() {
        if let Err(err) =
            crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global(
                &config.workspace_dir,
            )
        {
            // A concurrent init may have won the race and populated the registry,
            // in which case the `AlreadyInitialized`-style error is benign. But if
            // the registry is *still* `None`, init genuinely failed — fail fast
            // here rather than letting every downstream `spawn_agent` fail later
            // with `NoParentContext` after orchestration state has advanced.
            if crate::agent::harness::host_definition::AgentDefinitionRegistry::global().is_none() {
                return Err(err)
                    .context("initialize AgentDefinitionRegistry for orchestration root parent");
            }
            log::debug!(
                target: LOG_TARGET,
                "[parent_context] registry_init_raced err={err}"
            );
        }
    }

    let mut agent = OpenHumanSessionHost::from_config(ctx, config)
        .context("build Agent from config for orchestration root parent")?;

    let integrations = crate::integrations::composio::fetch_connected_integrations(config).await;
    agent.set_connected_integrations(integrations);

    Ok(ParentExecutionContext {
        agent_definition_id: agent_definition_id.to_string(),
        allowed_subagent_ids: HashSet::new(),
        turn_model_source: agent.turn_model_source(),
        all_tools: agent.tools_arc(),
        all_tool_specs: agent.durable_tool_specs_arc(),
        // No visibility filter for this spawned/background builder — empty means
        // "unknown" and callers fall back to the full registry (see field doc).
        visible_tool_names: HashSet::new(),
        // The agent's advertised surface, delegates included, for consumers
        // that describe what this parent can call.
        visible_tool_specs: agent.visible_tool_specs_arc(),
        subagent_tool_ceiling_names: HashSet::new(),
        model_name: agent.model_name().to_string(),
        temperature: agent.temperature(),
        workspace_dir: agent.workspace_dir().to_path_buf(),
        workspace_descriptor: None,
        agent_config: agent.agent_config().clone(),
        workflows: Arc::new(agent.workflows().to_vec()),
        memory_context: Arc::new(None),
        session_id: format!("{session_prefix}-{}", uuid::Uuid::new_v4()),
        channel: channel.to_string(),
        connected_integrations: agent.connected_integrations().to_vec(),
        tool_call_format: tinytools_agent::dialect::ToolCallFormat::PFormat,
        session_key: agent.session_key().to_string(),
        session_parent_prefix: agent.session_parent_prefix().map(str::to_string),
        on_progress: None,
        run_queue: None,
        host_env: agent.host_env().clone(),
    })
}

/// Build (or accept) the parent execution context for a **controller-spawned
/// background orchestration surface** that has no enclosing agent turn (the
/// workflow-run engine, the agent-team runtime, cron jobs), then run
/// `run(parent)` with it.
///
/// Folds [`build_root_parent`] and the hand-off into one call so a surface
/// cannot supply a hand-rolled parent, and cannot *forget* to supply one and
/// have every nested `spawn_subagent` die at runtime with
/// [`SubagentRunError::NoParentContext`](crate::agent::subagent_host::SubagentRunError::NoParentContext).
/// The parent is a value the closure receives and passes to the spawns it makes
/// (`AgentOrchestrationSession::spawn_agent`); nothing reads it ambiently.
///
/// `parent_override` is the explicit injection point for a parent the caller
/// already has (a test driving a surface under a mock parent, and thus a mock
/// provider, hermetically): when `Some`, it is used unchanged and no root is
/// built. Production surfaces pass `None` and a root is built from `config`.
///
/// Returns the closure's output on success, or the [`build_root_parent`] error
/// when the root context can't be constructed — the caller decides how to
/// degrade (fail the run, or fall back to an un-grounded path).
pub(crate) async fn with_root_parent<F, Fut>(
    ctx: &crate::core::runtime::DispatchCtx,
    parent_override: Option<ParentExecutionContext>,
    config: &Config,
    agent_definition_id: &str,
    channel: &str,
    session_prefix: &str,
    run: F,
) -> Result<Fut::Output>
where
    F: FnOnce(ParentExecutionContext) -> Fut,
    Fut: std::future::Future,
{
    let parent = match parent_override {
        Some(parent) => parent,
        None => {
            build_root_parent(ctx, config, agent_definition_id, channel, session_prefix).await?
        }
    };
    Ok(run(parent).await)
}

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
