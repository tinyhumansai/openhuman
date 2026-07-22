//! `rss-bench` — steady-state RSS benchmark for an embedded `openhuman_core`
//! agent roster (#5046).
//!
//! Mirrors the OpenCompany embedding contract: a bare [`Agent`] built directly
//! via [`Agent::builder`] (no `CoreBuilder`, no RPC, no background services)
//! with an injected mock provider, an in-process `"none"` memory backend, and a
//! per-agent temp workspace. Builds a 1-agent and an 8-agent roster, runs one
//! deterministic warm-up turn per agent to fault in lazy allocations, settles,
//! then samples `/proc/self/{status,smaps_rollup}`.
//!
//! Two modes:
//!   * `--child --roster N` builds one roster in a **fresh process**, warms up,
//!     settles, and prints one [`ProcSample`] JSON line. This is the isolated
//!     measured workload.
//!   * default (parent) re-execs itself `--repeat` times per roster size to get
//!     independent cold samples, aggregates, writes the raw JSON report
//!     (`--out`), and prints a human summary.
//!
//! Gated behind the default-OFF `rss-bench` feature so no benchmark code enters
//! the shipped desktop/library build. Build & run:
//! `cargo build --release --features rss-bench --bin rss-bench`.
//!
//! The pure sampling/aggregation logic lives in
//! [`openhuman_core::openhuman::proc_metrics`]; this binary is the fixture +
//! process driver.

use anyhow::{Context, Result};
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::inference::provider::{
    ChatRequest, ChatResponse, Provider, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::proc_metrics::{
    self, BenchReport, ProcSample, RosterResult, REPORT_SCHEMA_VERSION, RSS_BUDGET_KIB,
    RSS_HARD_CAP_KIB,
};
use openhuman_core::openhuman::tools::{Tool, ToolResult};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

/// Roster sizes measured by default: the 1-agent baseline and the
/// representative 8-agent company roster from #5046.
const DEFAULT_ROSTER_SIZES: &[usize] = &[1, 8];
/// Fresh processes sampled per roster size (≥ 5 per the issue).
const DEFAULT_REPEAT: usize = 5;
/// Per-child wall-clock budget. A child does bounded work (build a roster, one
/// warm-up turn, a ≤2 s settle), so anything beyond this is a stall — kill it and
/// fail the run rather than letting one bad child block the whole benchmark until
/// the outer CI job timeout.
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);

/// Provider that never touches the network: returns a fixed assistant message
/// with a `stop` shape (no tool calls) so a turn completes in one round-trip.
struct MockProvider;

#[async_trait]
impl Provider for MockProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("ok".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        Ok(ChatResponse {
            text: Some("ok".into()),
            tool_calls: vec![],
            usage: Some(UsageInfo {
                input_tokens: 8,
                output_tokens: 2,
                context_window: 8000,
                charged_amount_usd: 0.0,
                ..Default::default()
            }),
            reasoning_content: None,
        })
    }
}

/// Trivial host-supplied tool so the roster mirrors a real embedding (the host
/// injects its own tools). Never invoked — the provider returns no tool calls.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "echo"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("echo"))
    }
}

/// Zero-allocation no-op `Memory` for the fixture.
///
/// The benchmark measures a bare agent under the OpenCompany embedding contract
/// (host supplies its own `Memory` over its context store), *not* a memory
/// store. `create_memory(MemoryConfig{ backend: "none", .. })` does **not**
/// select a no-op backend — it always builds a `UnifiedMemory` (SQLite + the
/// default cloud embedder), which would inflate the measured RSS with
/// memory-store setup. Injecting a real no-op keeps the reading on the agent
/// harness itself.
struct NoopMemory;

#[async_trait]
impl Memory for NoopMemory {
    fn name(&self) -> &str {
        "noop"
    }
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }
    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }
    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }
    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }
    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }
    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }
    async fn count(&self) -> Result<usize> {
        Ok(0)
    }
    async fn health_check(&self) -> bool {
        true
    }
}

/// A built roster plus the temp workspaces that must outlive it — dropping the
/// `TempDir`s would delete the agents' workspaces mid-measurement.
struct Roster {
    agents: Vec<Agent>,
    _workspaces: Vec<TempDir>,
}

/// Build `n` bare agents, each with its own temp workspace, mock provider,
/// `"none"` memory backend, and a single host-supplied tool.
fn build_roster(n: usize) -> Result<Roster> {
    let mut agents = Vec::with_capacity(n);
    let mut workspaces = Vec::with_capacity(n);
    for i in 0..n {
        let workspace = TempDir::new().context("create temp workspace")?;
        let path = workspace.path().to_path_buf();

        let memory: Arc<dyn Memory> = Arc::new(NoopMemory);

        let agent = Agent::builder()
            .provider(Box::new(MockProvider))
            .tools(vec![Box::new(EchoTool)])
            .memory(memory)
            .tool_dispatcher(Box::new(NativeToolDispatcher))
            .model_name("bench-mock".into())
            .agent_definition_name(format!("bench-{i}"))
            .workspace_dir(path.clone())
            .action_dir(path)
            .auto_save(false)
            .build()
            .context("build bench agent")?;
        agents.push(agent);
        workspaces.push(workspace);
    }
    Ok(Roster {
        agents,
        _workspaces: workspaces,
    })
}

/// One deterministic warm-up turn per agent, forcing first-touch allocations
/// (prompt build, tokenizer, provider adapter) to fault in before measuring.
async fn warm_up(roster: &mut Roster) -> Result<()> {
    for agent in &mut roster.agents {
        let _ = agent.turn("warmup").await.context("warm-up turn")?;
    }
    Ok(())
}

/// Poll RSS until it stops climbing (Δ < 256 KiB between reads ~200 ms apart)
/// or a 2 s cap, draining async task-allocation jitter. No-op off Linux.
async fn settle() {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut last = proc_metrics::sample_self().map(|s| s.rss_kib).unwrap_or(0);
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let now = proc_metrics::sample_self()
            .map(|s| s.rss_kib)
            .unwrap_or(last);
        if now.abs_diff(last) < 256 || tokio::time::Instant::now() >= deadline {
            break;
        }
        last = now;
    }
}

/// Child mode: build one roster in this fresh process, warm up, settle, sample,
/// and print the sample as a single JSON line on stdout.
async fn run_child(roster_size: usize) -> Result<()> {
    // Diagnostics go to stderr so they never corrupt the single JSON line the
    // parent parses from stdout.
    eprintln!("[rss-bench] child: building {roster_size}-agent roster");
    let mut roster = build_roster(roster_size)?;
    eprintln!("[rss-bench] child: warming up {roster_size} agent(s)");
    warm_up(&mut roster).await?;
    eprintln!("[rss-bench] child: settling");
    settle().await;
    let sample = proc_metrics::sample_self()?;
    eprintln!(
        "[rss-bench] child: sampled rss={}KiB threads={}",
        sample.rss_kib, sample.threads
    );
    println!("{}", serde_json::to_string(&sample)?);
    drop(roster); // keep the roster alive until after the sample is taken
    Ok(())
}

/// Parent mode: re-exec the child `repeat` times per roster size, aggregate, and
/// write the report.
async fn run_parent(out: Option<PathBuf>, repeat: usize, roster_sizes: &[usize]) -> Result<()> {
    let exe = std::env::current_exe().context("resolve current exe")?;
    let mut rosters = Vec::with_capacity(roster_sizes.len());
    for &size in roster_sizes {
        let mut samples = Vec::with_capacity(repeat);
        for run in 0..repeat {
            eprintln!("[rss-bench] spawn child roster={size} run={run}");
            // `kill_on_drop` + a `timeout` around `wait_with_output` gives a
            // robust kill-and-reap: on timeout the cancelled future drops the
            // child, `kill_on_drop` sends SIGKILL, and the tokio runtime reaps it.
            let child = tokio::process::Command::new(&exe)
                .arg("--child")
                .arg("--roster")
                .arg(size.to_string())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .with_context(|| format!("spawn child roster={size} run={run}"))?;
            let output = match tokio::time::timeout(CHILD_TIMEOUT, child.wait_with_output()).await {
                Ok(res) => res.with_context(|| format!("await child roster={size} run={run}"))?,
                Err(_) => {
                    eprintln!(
                        "[rss-bench] child roster={size} run={run} timed out after {}s; killed",
                        CHILD_TIMEOUT.as_secs()
                    );
                    anyhow::bail!(
                        "child roster={size} run={run} timed out after {}s",
                        CHILD_TIMEOUT.as_secs()
                    );
                }
            };
            if !output.status.success() {
                eprintln!("[rss-bench] child roster={size} run={run} exited non-zero");
                anyhow::bail!(
                    "child roster={size} run={run} failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let line = stdout
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let sample: ProcSample = serde_json::from_str(line)
                .with_context(|| format!("parse child sample (roster={size}): {line:?}"))?;
            eprintln!(
                "[rss-bench] child roster={size} run={run} ok rss={}KiB",
                sample.rss_kib
            );
            samples.push(sample);
        }
        rosters.push(RosterResult::from_samples(size, samples));
    }

    let report = BenchReport {
        schema_version: REPORT_SCHEMA_VERSION,
        git_sha: git_sha(),
        kernel: kernel(),
        rss_budget_kib: RSS_BUDGET_KIB,
        rss_hard_cap_kib: RSS_HARD_CAP_KIB,
        rosters,
    };

    if let Some(path) = out {
        std::fs::write(&path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("write report to {}", path.display()))?;
    }
    println!("{}", proc_metrics::human_summary(&report));
    Ok(())
}

/// Best-effort commit id for the report header (`GITHUB_SHA` in CI).
fn git_sha() -> String {
    std::env::var("GITHUB_SHA").unwrap_or_else(|_| "unknown".into())
}

/// Best-effort kernel version for the report header.
fn kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| std::env::consts::OS.to_string())
}

/// Minimal flag parse — avoids a clap dependency for four flags.
struct Args {
    child: bool,
    roster: usize,
    repeat: usize,
    out: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut child = false;
    let mut roster = 1usize;
    let mut repeat = DEFAULT_REPEAT;
    let mut out = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--child" => child = true,
            "--roster" => {
                roster = it
                    .next()
                    .context("--roster needs a value")?
                    .parse()
                    .context("--roster value")?;
            }
            "--repeat" => {
                repeat = it
                    .next()
                    .context("--repeat needs a value")?
                    .parse()
                    .context("--repeat value")?;
            }
            "--out" => out = Some(PathBuf::from(it.next().context("--out needs a path")?)),
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(Args {
        child,
        roster,
        repeat,
        out,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    if args.child {
        run_child(args.roster).await
    } else {
        run_parent(args.out, args.repeat, DEFAULT_ROSTER_SIZES).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_roster_constructs_bare_agents_with_isolated_workspaces() {
        let roster = build_roster(8).expect("8-agent roster builds");
        assert_eq!(roster.agents.len(), 8);
        assert_eq!(roster._workspaces.len(), 8);
        // Each agent got a distinct workspace directory.
        let mut dirs: Vec<_> = roster
            ._workspaces
            .iter()
            .map(|w| w.path().to_path_buf())
            .collect();
        dirs.sort();
        dirs.dedup();
        assert_eq!(dirs.len(), 8, "workspaces must be isolated per agent");
    }

    #[tokio::test]
    async fn warm_up_turn_completes_without_network() {
        let mut roster = build_roster(1).expect("1-agent roster builds");
        warm_up(&mut roster).await.expect("warm-up turn completes");
        // The mock provider reports usage, so last_turn_usage is populated —
        // proving the embedding cost-metering contract works on the bare Agent.
        assert!(
            roster.agents[0].last_turn_usage().is_some(),
            "usage should be readable after a turn"
        );
    }
}
