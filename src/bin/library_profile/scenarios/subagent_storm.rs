//! `subagent-storm`: fuzz the *width* of delegation inside ONE core instance.
//!
//! One orchestrator turn fans out to **K** parallel researcher subagents (all
//! in-process tokio tasks, not child processes) via `spawn_parallel_agents`.
//! K comes from `OPENHUMAN_PROFILE_SUBAGENTS` (default 8; tested up to 32), and
//! each researcher carries per-subagent mock latency drawn from the shared
//! `OPENHUMAN_PROFILE_MOCK_LATENCY_MS` / `_JITTER_MS` knobs.
//!
//! ## Measurement shape (and a hard constraint we hit)
//!
//! The intended shape was: prewarm one width-K fan-out, then measure a second on
//! the same warm process so the delta is attributable to the K children rather
//! than cold bootstrap. That is **not achievable** here: the parallel-spawn
//! machinery is effectively one-shot per process. Once a fan-out's run ledger is
//! finalized, a second `spawn_parallel_agents` returns an empty result and the
//! orchestrator just re-calls the tool without re-running the workers. Worse,
//! merely *constructing* an orchestrator/researcher agent beforehand perturbs
//! the fan-out the same way. The only shape that reliably executes all K real
//! researcher subagents is a single fan-out as the process's first agent
//! activity.
//!
//! So this scenario measures exactly that: one cold width-K fan-out. `retained_delta_kib`
//! therefore includes the shared per-process bootstrap (~20–30 MiB every
//! scenario pays once) amortized across K, so a single run's
//! `marginal_rss_kib_per_agent = retained_delta_kib / K` is an **upper bound**,
//! not the true marginal. Read the true marginal by comparing runs: the fixed
//! bootstrap amortizes, so `(retained(K₂) - retained(K₁)) / (K₂ - K₁)` across
//! two widths (e.g. K=8 vs K=32) isolates the genuine per-additional-subagent
//! cost. `peak_delta_kib` additionally captures the K-concurrent peak.
//!
//! Reported fields: `subagents = K`, `marginal_rss_kib_per_agent` (retained/K,
//! upper bound), `checkpoints` (baseline → storm-turn-done), and
//! `turn_latency_ms` (percentiles across the K researcher child executions).
//! The workload asserts all K researcher subagents actually executed.

use anyhow::Result;
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;

use crate::harness::{fixture, measure, ProfileResult, TurnLatency};
use crate::mock::{subagent_marker, SubagentMock};

const DEFAULT_SUBAGENTS: usize = 8;

/// The orchestrator's top-level task. The mock ignores the wording and always
/// fans out to K researchers.
const STORM_PROMPT: &str = "Research every subsystem in parallel and merge the findings.";

/// Positive identity anchor for a researcher *worker* turn — its own system
/// prompt names it. Distinguishes a real worker from the orchestrator turns that
/// also echo every task marker in the fan-out tool call / result.
const RESEARCHER_IDENTITY: &str = "You are the **Researcher** agent";

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// Nearest-rank percentile of an already-sorted slice. `p` in `[0, 100]`.
fn percentile(sorted: &[u128], p: u128) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((p * sorted.len() as u128) + 99) / 100; // ceil(p% * n)
    let idx = rank.saturating_sub(1).min(sorted.len() as u128 - 1) as usize;
    sorted[idx]
}

fn latency_summary(mut samples: Vec<u128>) -> Option<TurnLatency> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    Some(TurnLatency {
        p50: percentile(&samples, 50),
        p95: percentile(&samples, 95),
        p99: percentile(&samples, 99),
        max: *samples.last().unwrap(),
    })
}

pub async fn run() -> Result<ProfileResult> {
    let width = env_usize("OPENHUMAN_PROFILE_SUBAGENTS", DEFAULT_SUBAGENTS);

    let mut fixture = fixture()?;
    // `spawn_parallel_agents` rejects a fan-out wider than the orchestrator's
    // `max_parallel_tools` (default 4). Raise it to K so the full width actually
    // spawns instead of erroring back to a re-spawn loop.
    fixture.config.agent.max_parallel_tools = width.max(4);
    let _ = init_global(256);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();

    let mock = SubagentMock::with_width(width);
    let _provider = test_provider_override::install_model(mock.clone());
    eprintln!("[library-profile] subagent-storm: width={width} — single cold width-K fan-out");

    // We drive the `orchestrator` agent directly: it owns `spawn_parallel_agents`
    // and allows the `researcher` subagent (the chat-tier subconscious has
    // neither, and would reject the fan-out). One orchestrator turn fans out to
    // K real researcher subagents via the parallel graph. This fan-out MUST be
    // the process's first agent activity — see the module docs for why prewarming
    // is not possible here.
    let config = fixture.config.clone();
    let mock_for_workload = mock.clone();
    let mut result = measure("subagent-storm", width, None, move |rec| async move {
        rec.checkpoint("baseline")?;
        let mut agent = Agent::from_config_for_agent(&config, "orchestrator")?;
        let reply = agent.run_single(STORM_PROMPT).await?;
        rec.checkpoint("storm-turn-done")?;
        anyhow::ensure!(!reply.trim().is_empty(), "empty storm-turn response");
        // Every one of the K researcher subagents must have actually executed as
        // its own worker turn: for each i there must be a prompt that carries the
        // researcher identity anchor AND that researcher's task marker — not
        // merely an orchestrator turn echoing every marker in the fan-out call.
        let prompts = mock_for_workload.prompts.lock().expect("mock prompt lock");
        for i in 1..=width {
            anyhow::ensure!(
                prompts
                    .iter()
                    .any(|p| p.contains(RESEARCHER_IDENTITY) && p.contains(&subagent_marker(i))),
                "researcher subagent {i}/{width} never executed as its own worker turn"
            );
        }
        Ok(())
    })
    .await?;

    // Marginal per subagent = retained / K. An UPPER BOUND: `retained_delta_kib`
    // still carries the one-time per-process bootstrap (see module docs), so the
    // true marginal is the cross-width delta `(retained(K₂)-retained(K₁))/(K₂-K₁)`.
    let marginal = if width > 0 {
        Some(result.retained_delta_kib as f64 / width as f64)
    } else {
        None
    };
    let latencies = mock
        .researcher_latencies_ms
        .lock()
        .expect("mock latency lock")
        .clone();
    eprintln!(
        "[library-profile] subagent-storm: width={width} retained_delta_kib={} \
         marginal_rss_kib_per_agent={:?} researcher_executions={}",
        result.retained_delta_kib,
        marginal,
        latencies.len()
    );

    result.subagents = Some(width);
    result.marginal_rss_kib_per_agent = marginal;
    result.turn_latency_ms = latency_summary(latencies);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_nearest_rank() {
        let v: Vec<u128> = (1..=100).collect();
        assert_eq!(percentile(&v, 50), 50);
        assert_eq!(percentile(&v, 95), 95);
        assert_eq!(percentile(&v, 99), 99);
        assert_eq!(percentile(&v, 100), 100);
        assert_eq!(percentile(&[], 50), 0);
    }

    #[test]
    fn latency_summary_none_when_empty() {
        assert!(latency_summary(Vec::new()).is_none());
        let s = latency_summary(vec![10, 20, 30]).unwrap();
        assert_eq!(s.max, 30);
        assert_eq!(s.p50, 20);
    }

    #[test]
    fn env_usize_falls_back_on_zero_or_unset() {
        assert_eq!(env_usize("OPENHUMAN_PROFILE_STORM_UNSET_XYZ", 8), 8);
    }
}
