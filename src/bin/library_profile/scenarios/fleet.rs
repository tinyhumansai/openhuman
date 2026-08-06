//! `fleet`: can OpenHuman host 100–1000 live agents on a 2 GB / 2 vCPU box?
//!
//! Answers four questions in one process: (1) marginal RSS per live agent
//! (`baseline → constructed`), (2) idle CPU of parked agents (CPU delta over a
//! 10 s do-nothing window), (3) thread + fd growth vs N (rides along in
//! `ProcSample`), and (4) turn latency percentiles under overlapping load.
//!
//! Env knobs:
//! - `OPENHUMAN_PROFILE_AGENTS` (default 100) — live agents to construct.
//! - `OPENHUMAN_PROFILE_TURNS` (default 3) — turns per agent under load.
//! - `OPENHUMAN_PROFILE_MOCK_LATENCY_MS` / `_JITTER_MS` — mock reply latency.
//! - `OPENHUMAN_PROFILE_TARGET_AGENTS` (default 1000) — budget projection target.
//! - `OPENHUMAN_PROFILE_RAM_BUDGET_MIB` (default 2048) — budget ceiling.
//!
//! `OPENHUMAN_PROFILE_WORKER_THREADS` is honoured in `main` (runtime built
//! manually) rather than here.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;
use openhuman_core::openhuman::platform::proc_metrics;

use crate::harness::{fixture, measure, FleetBudget, ProfileResult, Recorder, TurnLatency};
use crate::mock::LatencyMock;

const DEFAULT_AGENTS: usize = 100;
const DEFAULT_TURNS: usize = 3;
const DEFAULT_TARGET_AGENTS: u64 = 1000;
const DEFAULT_RAM_BUDGET_MIB: u64 = 2048;
const IDLE_WINDOW: Duration = Duration::from_secs(10);

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// Raise `RLIMIT_NOFILE` toward its hard cap so N agents (each opening SQLite
/// etc.) don't exhaust the default macOS 256 soft limit. Logs old/new to stderr.
fn raise_fd_limit() {
    use std::mem::MaybeUninit;
    let mut lim = MaybeUninit::<libc::rlimit>::uninit();
    // SAFETY: `getrlimit` initialises `lim` on success.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, lim.as_mut_ptr()) } != 0 {
        eprintln!("[library-profile] fleet: getrlimit(RLIMIT_NOFILE) failed");
        return;
    }
    // SAFETY: initialised by the successful `getrlimit`.
    let mut lim = unsafe { lim.assume_init() };
    let old_soft = lim.rlim_cur;
    lim.rlim_cur = lim.rlim_max;
    // SAFETY: raising the soft limit to the existing hard limit is always valid.
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lim) };
    eprintln!(
        "[library-profile] fleet: RLIMIT_NOFILE soft {old_soft} -> {} (hard {}) setrlimit_rc={rc}",
        lim.rlim_cur, lim.rlim_max
    );
}

/// Shared metrics captured inside the measured closure and read back after.
#[derive(Default)]
struct FleetMetrics {
    agents_built: usize,
    baseline_rss_kib: u64,
    constructed_rss_kib: u64,
    idle_cpu_ms: u64,
    latency_ms: Vec<u128>,
}

/// Percentile (nearest-rank) of an already-sorted slice. `p` in `[0,100]`.
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

/// Build agents sequentially, checkpointing the marginal-RSS curve. On a
/// mid-construction failure, records `construction-failed-<count>` and returns
/// what was built rather than crashing.
fn build_agents(
    config: &openhuman_core::openhuman::config::Config,
    n: usize,
    rec: &Recorder,
) -> Result<Vec<Agent>> {
    let stride = (n / 10).max(1);
    let mut agents = Vec::with_capacity(n);
    for i in 0..n {
        match Agent::from_config_for_agent(config, "subconscious") {
            Ok(agent) => agents.push(agent),
            Err(err) => {
                eprintln!(
                    "[library-profile] fleet: construction failed at agent {} — {err}",
                    i + 1
                );
                rec.checkpoint(format!("construction-failed-{}", agents.len()))?;
                break;
            }
        }
        let built = i + 1;
        if built % stride == 0 || built == n {
            rec.checkpoint(format!("built-{built}"))?;
        }
    }
    Ok(agents)
}

pub async fn run() -> Result<ProfileResult> {
    let agents_requested = env_usize("OPENHUMAN_PROFILE_AGENTS", DEFAULT_AGENTS);
    let turns = env_usize("OPENHUMAN_PROFILE_TURNS", DEFAULT_TURNS);
    let target_agents = env_u64("OPENHUMAN_PROFILE_TARGET_AGENTS", DEFAULT_TARGET_AGENTS);
    let ram_budget_mib = env_u64("OPENHUMAN_PROFILE_RAM_BUDGET_MIB", DEFAULT_RAM_BUDGET_MIB);

    raise_fd_limit();

    let fixture = fixture()?;
    let _ = init_global(256);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let mock = LatencyMock::from_env("Fleet agent: nothing needs your attention.");
    let _provider = test_provider_override::install_model(mock.clone());
    eprintln!(
        "[library-profile] fleet: agents={agents_requested} turns={turns} \
         target={target_agents} budget_mib={ram_budget_mib}"
    );

    let metrics = Arc::new(Mutex::new(FleetMetrics::default()));
    let config = fixture.config.clone();
    let metrics_for_workload = Arc::clone(&metrics);

    let mut result = measure(
        "fleet",
        agents_requested,
        Some(turns),
        move |rec| async move {
            rec.checkpoint("baseline")?;
            let baseline_rss = proc_metrics::sample_self()?.rss_kib;

            // (b) construct N agents sequentially, curve visible via checkpoints.
            let mut agents = build_agents(&config, agents_requested, &rec)?;
            let agents_built = agents.len();
            rec.checkpoint("constructed")?;
            let constructed_rss = proc_metrics::sample_self()?.rss_kib;

            {
                let mut m = metrics_for_workload.lock().expect("metrics lock");
                m.agents_built = agents_built;
                m.baseline_rss_kib = baseline_rss;
                m.constructed_rss_kib = constructed_rss;
            }
            eprintln!(
                "[library-profile] fleet: built {agents_built}/{agents_requested} agents; \
             baseline_rss={baseline_rss} constructed_rss={constructed_rss}"
            );

            // (d) idle phase — park all agents, measure CPU drift over 10 s.
            rec.checkpoint("idle-start")?;
            let idle_start = proc_metrics::sample_self()?;
            tokio::time::sleep(IDLE_WINDOW).await;
            rec.checkpoint("idle-end")?;
            let idle_end = proc_metrics::sample_self()?;
            let idle_cpu_ms = (idle_end.cpu_user_ms + idle_end.cpu_system_ms)
                .saturating_sub(idle_start.cpu_user_ms + idle_start.cpu_system_ms);
            metrics_for_workload
                .lock()
                .expect("metrics lock")
                .idle_cpu_ms = idle_cpu_ms;
            eprintln!("[library-profile] fleet: idle CPU over 10s = {idle_cpu_ms} ms");

            // (e) load phase — TURNS turns per agent, one task each, staggered.
            let mut handles = Vec::with_capacity(agents.len());
            for (idx, mut agent) in agents.drain(..).enumerate() {
                let stagger = Duration::from_millis(((idx as u64) * 10).min(2000));
                handles.push(tokio::spawn(async move {
                    tokio::time::sleep(stagger).await;
                    let mut latencies = Vec::with_capacity(turns);
                    for _ in 0..turns {
                        let started = Instant::now();
                        let reply = agent.run_single("Give me a one-line status update.").await;
                        let elapsed = started.elapsed().as_millis();
                        match reply {
                            Ok(text) if !text.trim().is_empty() => latencies.push(elapsed),
                            Ok(_) => eprintln!("[library-profile] fleet: empty reply agent={idx}"),
                            Err(err) => {
                                eprintln!("[library-profile] fleet: turn error agent={idx} — {err}")
                            }
                        }
                    }
                    latencies
                }));
            }

            let mut all_latencies = Vec::new();
            for handle in handles {
                match handle.await {
                    Ok(mut latencies) => all_latencies.append(&mut latencies),
                    Err(err) => eprintln!("[library-profile] fleet: task join error — {err}"),
                }
            }
            metrics_for_workload
                .lock()
                .expect("metrics lock")
                .latency_ms = all_latencies;
            rec.checkpoint("load-done")?;
            Ok(())
        },
    )
    .await?;

    // Fold fleet-specific metrics into the pinned result.
    let metrics = Arc::try_unwrap(metrics)
        .map(|m| m.into_inner().expect("metrics lock"))
        .unwrap_or_default();

    let agents_built = metrics.agents_built;
    let marginal = if agents_built > 0 {
        Some(
            (metrics.constructed_rss_kib as f64 - metrics.baseline_rss_kib as f64)
                / agents_built as f64,
        )
    } else {
        None
    };

    // (4) budget projection: settled base + marginal * target.
    let base_mib = metrics.baseline_rss_kib as f64 / 1024.0;
    let marginal_mib = marginal.unwrap_or(0.0) / 1024.0;
    let projected = base_mib + marginal_mib * target_agents as f64;
    let budget = FleetBudget {
        target_agents,
        ram_budget_mib,
        projected_rss_mib_at_target: projected,
        fits: projected <= ram_budget_mib as f64,
    };

    result.agents = Some(agents_requested);
    result.agents_built = Some(agents_built);
    result.marginal_rss_kib_per_agent = marginal;
    result.idle_cpu_ms = Some(metrics.idle_cpu_ms);
    result.turn_latency_ms = latency_summary(metrics.latency_ms);
    result.budget = Some(budget);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_nearest_rank() {
        let mut v: Vec<u128> = (1..=100).collect();
        v.sort_unstable();
        assert_eq!(percentile(&v, 50), 50);
        assert_eq!(percentile(&v, 95), 95);
        assert_eq!(percentile(&v, 99), 99);
        assert_eq!(percentile(&v, 100), 100);
        assert_eq!(percentile(&[], 50), 0);
        assert_eq!(percentile(&[7], 99), 7);
    }

    #[test]
    fn latency_summary_none_when_empty() {
        assert!(latency_summary(Vec::new()).is_none());
        let s = latency_summary(vec![10, 20, 30]).unwrap();
        assert_eq!(s.max, 30);
        assert_eq!(s.p50, 20);
    }
}
