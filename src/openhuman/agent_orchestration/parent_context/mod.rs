//! Shared root [`ParentExecutionContext`] builder for controller-spawned
//! orchestration tasks (#3374 PR4, extracted from the #3375 workflow engine).
//!
//! Both the workflow-run engine ([`workflow_runs::engine`]) and the agent-team
//! runtime ([`agent_teams::runtime`]) need to spawn real sub-agents from a
//! background task that has **no** enclosing agent turn on the stack. Those
//! spawns read their parent execution context from a task-local
//! ([`current_parent`]) that is only set inside an agent turn — so a naive spawn
//! fails with `NoParentContext`.
//!
//! The fix (proven in `triage::escalation::dispatch_target_agent`) is to build a
//! *root* [`ParentExecutionContext`] from a config-built [`Agent`] and run the
//! whole loop inside [`with_parent_context`]. Every nested `spawn_agent` then
//! resolves `current_parent()` to this root, inheriting a real provider, tool
//! registry, memory, and model — the same construction path `agent_chat` uses.
//!
//! This was originally inlined in the workflow engine; PR4 lifts it here so the
//! team runtime reuses the exact same construction (and the single
//! registry-initialisation defense) rather than carrying a second copy of the
//! ~20-field context literal that could drift.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::openhuman::agent::harness::fork_context::ParentExecutionContext;
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;

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
    if crate::openhuman::agent::harness::AgentDefinitionRegistry::global().is_none() {
        if let Err(err) = crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(
            &config.workspace_dir,
        ) {
            // A concurrent init may have won the race and populated the registry,
            // in which case the `AlreadyInitialized`-style error is benign. But if
            // the registry is *still* `None`, init genuinely failed — fail fast
            // here rather than letting every downstream `spawn_agent` fail later
            // with `NoParentContext` after orchestration state has advanced.
            if crate::openhuman::agent::harness::AgentDefinitionRegistry::global().is_none() {
                return Err(err)
                    .context("initialize AgentDefinitionRegistry for orchestration root parent");
            }
            log::debug!(
                target: LOG_TARGET,
                "[parent_context] registry_init_raced err={err}"
            );
        }
    }

    let mut agent = Agent::from_config(config)
        .context("build Agent from config for orchestration root parent")?;

    let integrations = crate::openhuman::composio::fetch_connected_integrations(config).await;
    agent.set_connected_integrations(integrations);

    Ok(ParentExecutionContext {
        agent_definition_id: agent_definition_id.to_string(),
        allowed_subagent_ids: HashSet::new(),
        provider: agent.provider_arc(),
        all_tools: agent.tools_arc(),
        all_tool_specs: agent.tool_specs_arc(),
        // No visibility filter for this spawned/background builder — empty means
        // "unknown" and callers fall back to the full registry (see field doc).
        visible_tool_names: HashSet::new(),
        model_name: agent.model_name().to_string(),
        temperature: agent.temperature(),
        workspace_dir: agent.workspace_dir().to_path_buf(),
        memory: agent.memory_arc(),
        agent_config: agent.agent_config().clone(),
        workflows: Arc::new(agent.workflows().to_vec()),
        memory_context: Arc::new(None),
        session_id: format!("{session_prefix}-{}", uuid::Uuid::new_v4()),
        channel: channel.to_string(),
        connected_integrations: agent.connected_integrations().to_vec(),
        tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::PFormat,
        session_key: agent.session_key().to_string(),
        session_parent_prefix: agent.session_parent_prefix().map(str::to_string),
        on_progress: None,
        run_queue: None,
    })
}
