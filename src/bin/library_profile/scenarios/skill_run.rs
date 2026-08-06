//! `skill-run`: the true, process-*tree* cost of a skill step that executes on
//! a real language runtime — the interpreter child process included — and the
//! regression instrument for the shared runtime pool (issue #5106).
//!
//! ## What it actually runs
//!
//! A skill run's orchestrator (`spawn_workflow_run_background` → the
//! `orchestrator` agent) deliberately owns **no** `node_exec` tool: the
//! chat-tier orchestrator never executes code itself, it delegates every code
//! step to the `code_executor` specialist (the only builtin whose allow-list
//! carries `node_exec` / `npm_exec`). So the agent that genuinely spawns the
//! language runtime *is* `code_executor`. This scenario drives that specialist
//! directly — one turn, one scripted `node_exec` call — which is the real
//! node-executing path, not a bare `std::process` spawn.
//!
//! ## Concurrency knob (K parallel skill runs)
//!
//! `OPENHUMAN_PROFILE_SKILL_RUN_CONCURRENCY=K` (default 1) drives **K**
//! `code_executor` turns in parallel, each emitting its own `node_exec` call.
//! The point of #5106: with the runtime pool **on**, K concurrent skill runs
//! share a small bounded set of warm `node` workers, so the process tree grows
//! by ~one pooled worker — **not** K interpreters. This scenario asserts that
//! (`tree.child_count <= max_workers`) whenever pooling is on and K > 1, so a
//! regression that reintroduces per-run forking fails the profiling suite.
//!
//! Toggle for an A/B baseline:
//!
//! * `OPENHUMAN_PROFILE_SKILL_RUN_POOL=off` — disable the pool (legacy per-call
//!   spawn); the tree then shows ~K resident `node` children at peak.
//! * `OPENHUMAN_PROFILE_SKILL_RUN_POOL_WORKERS=W` — pool size (default 1, for a
//!   tight, deterministic bound).
//!
//! ## No interpreter download
//!
//! `node.prefer_system = true` (the default) means a host `node` whose **major**
//! matches the configured target is reused rather than downloaded. This
//! scenario **requires** a system `node` and bails with a clear stderr error +
//! nonzero exit if none is on `PATH` — it must never pull a runtime.
//!
//! The measured cost lands in `result.tree` (`tree_rss_kib`, `child_count`,
//! per-child RSS), captured at the workload peak.

use anyhow::{Context, Result};
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;
use openhuman_core::openhuman::security::AutonomyLevel;

use crate::harness::{fixture, measure_with_tree, EnvGuard, ProfileResult};
use crate::mock::SkillRunMock;

/// The specialist agent that owns `node_exec` and spawns the runtime child.
const CODE_AGENT: &str = "code_executor";

/// Probe for a usable system `node`. Returns its version on success; on failure
/// prints a clear `[library-profile]` stderr error and returns `Err` (which
/// propagates to a nonzero process exit). We must NOT download an interpreter.
fn require_system_node() -> Result<String> {
    match std::process::Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            eprintln!("[library-profile] skill-run: using system node {version}");
            Ok(version)
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!(
                "[library-profile] skill-run: `node --version` failed (status {:?}): {stderr}",
                output.status.code()
            );
            anyhow::bail!("skill-run requires a working system `node`, but `node --version` failed")
        }
        Err(err) => {
            eprintln!(
                "[library-profile] skill-run: `node` not found on PATH: {err}. Install Node.js — \
                 the profiler will NOT download an interpreter."
            );
            anyhow::bail!("skill-run requires a system `node` on PATH; none found")
        }
    }
}

/// Read a `>= 1` usize from the environment. Absent ⇒ `default`; present but not
/// a positive integer ⇒ a hard error (silently coercing a bad `0`/garbage value
/// would change the workload and bypass the `K > 1` pool gate).
fn env_usize(key: &str, default: usize) -> Result<usize> {
    match std::env::var(key) {
        Err(_) => Ok(default),
        Ok(raw) => {
            let n: usize = raw
                .trim()
                .parse()
                .with_context(|| format!("{key}={raw:?} is not a valid integer"))?;
            anyhow::ensure!(n >= 1, "{key}={raw:?} must be >= 1");
            Ok(n)
        }
    }
}

/// Apply the `[runtime_pool]` settings to the in-memory fixture config. The exec
/// tools snapshot `config.runtime_pool` at construction (the pool config is
/// injected, never re-read from disk on the hot path), so mutating the config
/// the agent is built from is what toggles pooling for the scenario.
fn apply_pool_config(config: &mut Config, pool_enabled: bool, workers: usize) {
    config.runtime_pool.enabled = pool_enabled;
    config.runtime_pool.node.max_workers = workers;
    config.runtime_pool.node.idle_ttl_secs = 300;
    config.runtime_pool.node.recycle_after_jobs = 0;
}

pub async fn run() -> Result<ProfileResult> {
    // Hard requirement: a system node must be present (no download).
    require_system_node()?;

    let concurrency = env_usize("OPENHUMAN_PROFILE_SKILL_RUN_CONCURRENCY", 1)?;
    let pool_enabled = std::env::var("OPENHUMAN_PROFILE_SKILL_RUN_POOL")
        .map(|v| !v.trim().eq_ignore_ascii_case("off"))
        .unwrap_or(true);
    let pool_workers = env_usize("OPENHUMAN_PROFILE_SKILL_RUN_POOL_WORKERS", 1)?;

    // `node_exec` is a Write-class acting tool. Full autonomy keeps the gate
    // from parking the turn on approval; the gate is also opted out explicitly.
    let mut fixture = fixture()?;
    fixture.config.autonomy.level = AutonomyLevel::Full;
    apply_pool_config(&mut fixture.config, pool_enabled, pool_workers);
    let _approval_env = EnvGuard::set("OPENHUMAN_APPROVAL_GATE", "0");

    let _ = init_global(256);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();

    let mock = SkillRunMock::new();
    let _provider = test_provider_override::install_model(mock.clone());
    eprintln!(
        "[library-profile] skill-run: registries ready, node_exec mock installed \
         (agent={CODE_AGENT}, concurrency={concurrency}, pool={}, pool_workers={pool_workers})",
        if pool_enabled { "on" } else { "off" }
    );

    let mock_for_workload = mock.clone();
    let config = fixture.config.clone();
    let mut result = measure_with_tree("skill-run", concurrency, None, move |rec| async move {
        rec.checkpoint("turn-start")?;
        // Drive K code_executor turns concurrently. `join_all` gives real
        // process-level parallelism (each node_exec awaits its own child /
        // pooled job) without requiring the agent future to be `Send`.
        let futures = (0..concurrency).map(|idx| {
            let config = config.clone();
            async move {
                let mut agent = Agent::from_config_for_agent(&config, CODE_AGENT)
                    .with_context(|| format!("building code_executor agent #{idx}"))?;
                let reply = agent
                    .run_single(
                        "Run a short JavaScript computation with node_exec and report the JSON it prints.",
                    )
                    .await
                    .with_context(|| format!("code_executor turn #{idx}"))?;
                anyhow::ensure!(!reply.trim().is_empty(), "empty code_executor reply #{idx}");
                Ok::<(), anyhow::Error>(())
            }
        });
        let outcomes = futures::future::join_all(futures).await;
        for outcome in outcomes {
            outcome?;
        }
        rec.checkpoint("turn-done")?;
        anyhow::ensure!(
            mock_for_workload.node_call_emitted(),
            "the node_exec tool call was never emitted"
        );
        anyhow::ensure!(
            mock_for_workload.node_output_seen(),
            "the node child's output never flowed back — the interpreter child did not run/print"
        );
        Ok(())
    })
    .await?;

    match result.tree.as_ref() {
        Some(tree) if tree.child_count >= 1 => {
            eprintln!(
                "[library-profile] skill-run: captured tree_rss_kib={} child_count={} \
                 concurrency={concurrency} pool={} children={:?}",
                tree.tree_rss_kib,
                tree.child_count,
                if pool_enabled { "on" } else { "off" },
                tree.children
            );
        }
        Some(tree) => {
            eprintln!(
                "[library-profile] skill-run: tree captured but no child resident at peak \
                 (tree_rss_kib={}). With the pool on this is expected between jobs; with the \
                 pool off the node child may have been too short-lived.",
                tree.tree_rss_kib
            );
        }
        None => {
            eprintln!("[library-profile] skill-run: WARNING no process-tree sample captured");
        }
    }

    // DoD gate (#5106): with pooling on, K concurrent skill runs must not fork K
    // interpreters — the tree grows by ~one pooled worker. A regression that
    // reintroduces per-run forking makes child_count scale with K and fails here.
    if pool_enabled && concurrency > 1 {
        // Require a real measurement: a missing tree or zero children would let
        // the gate "pass" without ever observing an interpreter. With the
        // scenario's 300 s idle TTL the pooled worker is resident at sample time,
        // so this must hold.
        let tree = result.tree.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "pool gate: no process-tree sample captured for K={concurrency}; cannot verify \
                 the pooled worker (tree sampling is required for this gate)"
            )
        })?;
        anyhow::ensure!(
            tree.child_count >= 1,
            "pool gate: process tree captured zero interpreter children at K={concurrency}; \
             the pooled worker was not observed"
        );
        anyhow::ensure!(
            tree.child_count <= pool_workers,
            "runtime pool failed to bound interpreters: child_count={} exceeds max_workers={} \
             at concurrency K={} — expected warm-worker reuse, not K forked children",
            tree.child_count,
            pool_workers,
            concurrency
        );
        eprintln!(
            "[library-profile] skill-run: POOL OK — {} node worker(s) served {concurrency} \
             concurrent skill runs (would be ~{concurrency} interpreters unpooled)",
            tree.child_count
        );
    }

    // Fold in scenario-visible fields (schema stays additive).
    result.workload_units = concurrency;
    Ok(result)
}
