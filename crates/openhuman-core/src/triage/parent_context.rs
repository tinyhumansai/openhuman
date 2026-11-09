//! Root [`ParentExecutionContext`] builder for triage escalation.
//!
//! Triage escalation has no enclosing agent turn on the stack, so there is no
//! live parent execution context for its sub-agent dispatch to inherit. This
//! builds the root context from a config-built [`Agent`] using the same path as
//! `agent_chat`, including a real provider, tool registry, memory, and model.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::agent::subagent_host::ParentExecutionContext;
use crate::agent::OpenHumanSessionHost;
use crate::config::Config;

const LOG_TARGET: &str = "agent_orchestration::parent_context";

/// Build a root [`ParentExecutionContext`] from a config-built [`Agent`].
///
/// Mirrors `triage::escalation::dispatch_target_agent` — the proven path for
/// running sub-agents without an enclosing agent turn. Triage supplies its
/// identity fields (`"triage"` for the definition, channel, and session prefix),
/// which labels the root parent in spawn metadata and namespaces its session.
/// Every other field is inherited verbatim from the config-built agent, so the
/// spawned child behaves exactly like a normal sub-agent dispatch.
pub(crate) async fn build_root_parent(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    agent_definition_id: &str,
    channel: &str,
    session_prefix: &str,
) -> Result<ParentExecutionContext> {
    // Sub-agent spawns resolve their definition through the global
    // agent-definition registry, so it MUST be initialised before any spawn.
    // The full runtime boot (`bootstrap_core_runtime`) does this, but triage can
    // also be reached from contexts that only built the HTTP router
    // (e.g. the JSON-RPC e2e harness) — so init defensively here. `OnceLock`
    // makes this idempotent: a no-op when the registry is already loaded.
    if crate::agent_registry::host_definition::AgentDefinitionRegistry::global().is_none() {
        if let Err(err) =
            crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global(
                &config.workspace_dir,
            )
        {
            // A concurrent init may have won the race and populated the registry,
            // in which case the `AlreadyInitialized`-style error is benign. But if
            // the registry is *still* `None`, init genuinely failed — fail fast
            // here rather than letting every downstream `spawn_agent` fail later
            // with `NoParentContext` after orchestration state has advanced.
            if crate::agent_registry::host_definition::AgentDefinitionRegistry::global().is_none() {
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
        .await
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
