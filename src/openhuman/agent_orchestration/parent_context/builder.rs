//! Shared root [`ParentExecutionContext`] builder for controller-spawned
//! orchestration tasks (#3374 PR4, extracted from the #3375 workflow engine).
//!
//! The workflow-run engine ([`workflow_runs::engine`]), the agent-team runtime
//! ([`agent_teams::runtime`]), and the subconscious tick
//! ([`crate::openhuman::subconscious`]) all need to spawn real sub-agents from a
//! background task that has **no** enclosing agent turn on the stack. Those
//! spawns read their parent execution context from a task-local
//! ([`current_parent`]) that is only set inside an agent turn — so a naive spawn
//! fails with `NoParentContext` (the TAURI-RUST-HMW regression, #4337).
//!
//! The fix (proven in `triage::escalation::dispatch_target_agent`) is to build a
//! *root* [`ParentExecutionContext`] from a config-built [`Agent`] and run the
//! whole loop inside [`with_parent_context`]. Every nested `spawn_agent` then
//! resolves `current_parent()` to this root, inheriting a real provider, tool
//! registry, memory, and model — the same construction path `agent_chat` uses.
//!
//! [`with_root_parent`] is the single blessed entry point that folds the build +
//! install into one call, so a surface can neither hand-roll the parent nor
//! forget to install it. Every background orchestration surface goes through it.
//!
//! This was originally inlined in the workflow engine; #3374 PR4 lifted it here
//! so each surface reuses the exact same construction (and the single
//! registry-initialisation defense) rather than carrying a second copy of the
//! ~20-field context literal that could drift.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::openhuman::agent::harness::fork_context::{
    current_parent, with_parent_context, ParentExecutionContext,
};
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
        turn_model_source: agent.turn_model_source(),
        all_tools: agent.tools_arc(),
        all_tool_specs: agent.tool_specs_arc(),
        // No visibility filter for this spawned/background builder — empty means
        // "unknown" and callers fall back to the full registry (see field doc).
        visible_tool_names: HashSet::new(),
        model_name: agent.model_name().to_string(),
        temperature: agent.temperature(),
        workspace_dir: agent.workspace_dir().to_path_buf(),
        workspace_descriptor: None,
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

/// Ensure a parent execution context is installed for `fut`, then run it — the
/// single blessed entry point for **controller-spawned background orchestration
/// surfaces** that have no enclosing agent turn (the workflow-run engine, the
/// agent-team runtime, the subconscious tick).
///
/// Folds [`build_root_parent`] + [`with_parent_context`] into one call so a
/// surface cannot install a hand-rolled parent, and — the TAURI-RUST-HMW
/// failure mode — cannot *forget* to install one at all and have every nested
/// `spawn_subagent` die at runtime with
/// [`SubagentRunError::NoParentContext`](crate::openhuman::agent::harness::subagent_runner::SubagentRunError::NoParentContext).
/// Running a background surface and establishing its root context become the
/// same act.
///
/// When an ambient [`current_parent`] is already installed, `fut` runs under it
/// unchanged rather than building a second root. In production these surfaces
/// run on freshly-spawned tasks where task-locals never cross the `tokio::spawn`
/// boundary, so the ambient parent is always absent and a root is built from
/// `config` — but reusing an installed parent keeps the helper correct if it is
/// ever nested inside a turn, and lets tests drive a surface under a mock
/// parent (and thus a mock provider) hermetically.
///
/// Returns the future's output on success, or the [`build_root_parent`] error
/// when the root context can't be constructed — the caller decides how to
/// degrade (fail the run, or fall back to an un-grounded path).
///
/// Only for surfaces that build their parent *from `Config`* because there is
/// no ambient parent. Spawn sites that re-install an **inherited**
/// `current_parent()` across a `tokio::spawn` boundary (e.g.
/// `spawn_async_subagent`, `spawn_worker_thread`, the orchestration `ops` task)
/// are a different pattern and call [`with_parent_context`] directly.
pub(crate) async fn with_root_parent<F>(
    config: &Config,
    agent_definition_id: &str,
    channel: &str,
    session_prefix: &str,
    fut: F,
) -> Result<F::Output>
where
    F: std::future::Future,
{
    // Already inside a turn (nested call, or a test harness installed a mock
    // parent): reuse it rather than building a second root.
    if current_parent().is_some() {
        return Ok(fut.await);
    }
    let parent = build_root_parent(config, agent_definition_id, channel, session_prefix).await?;
    Ok(with_parent_context(parent, fut).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::fork_context::current_parent;

    fn test_config() -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Config {
            workspace_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        (dir, config)
    }

    /// Baseline for the bug: with no enclosing agent turn there is no ambient
    /// parent — exactly the state the subconscious tick spawned `context_scout`
    /// in (TAURI-RUST-HMW / #4337), which made `run_subagent` return
    /// `NoParentContext`.
    #[tokio::test]
    async fn no_ambient_parent_outside_with_root_parent() {
        assert!(
            current_parent().is_none(),
            "no parent context should be installed by default"
        );
    }

    /// Regression (TAURI-RUST-HMW / #4337): `with_root_parent` must install a
    /// real parent for the wrapped future so a background orchestration surface
    /// (subconscious tick, workflow engine, team runtime) can spawn sub-agents
    /// without hitting `NoParentContext`. Proven by observing the installed
    /// parent from inside the future.
    #[tokio::test]
    async fn with_root_parent_installs_parent_for_inner_future() {
        let (_dir, config) = test_config();
        let observed = with_root_parent(
            &config,
            "subconscious",
            "subconscious",
            "subconscious",
            async { current_parent().map(|p| p.agent_definition_id) },
        )
        .await
        .expect("root parent builds from config");
        assert_eq!(
            observed.as_deref(),
            Some("subconscious"),
            "inner future must observe the installed root parent"
        );
    }

    /// When a parent is already installed, `with_root_parent` reuses it instead
    /// of building a second root — so a surface nested in a turn (or a test
    /// driving it under a mock parent) runs under the ambient context.
    #[tokio::test]
    async fn with_root_parent_reuses_ambient_parent() {
        let (_dir, config) = test_config();
        let outer = build_root_parent(&config, "outer", "outer", "outer")
            .await
            .expect("build ambient parent");
        let observed = with_parent_context(outer, async {
            with_root_parent(&config, "inner", "inner", "inner", async {
                current_parent().map(|p| p.agent_definition_id)
            })
            .await
            .expect("reuses ambient, no build error")
        })
        .await;
        assert_eq!(
            observed.as_deref(),
            Some("outer"),
            "with_root_parent must reuse the ambient parent, not build a new 'inner' root"
        );
    }
}
