//! Title: Linux agent fleet memory and latency
//! Summary: Measure retained runtime-owned agents using loopback inference and two worker threads.
//! Run: offline on Linux; use a fresh constrained cgroup for release measurements.
//! Profile: release
//! Feature: default
//! Default features: disabled

#![recursion_limit = "512"]

use std::time::Instant;

use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, Provider, Runtime, ToolScopeSpec, Workspace,
};
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn rss_kib() -> anyhow::Result<u64> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let line = status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .ok_or_else(|| anyhow::anyhow!("VmRSS missing"))?;
    Ok(line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("VmRSS value missing"))?
        .parse()?)
}

// ANCHOR: linux-fleet
fn main() -> anyhow::Result<()> {
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "100".into())
        .parse()?;
    anyhow::ensure!(count > 0, "agent count must be positive");
    openhuman_embed::process::tokio_runtime_builder()
        .worker_threads(2)
        .build()?
        .block_on(measure(count))?;
    println!("EXAMPLE_OK linux_fleet");
    Ok(())
}
// ANCHOR_END: linux-fleet

async fn measure(count: usize) -> anyhow::Result<()> {
    let mock = MockServer::builder()
        .disable_request_recording()
        .start()
        .await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":"linux-fleet", "object":"chat.completion", "created":1700000000,
            "model":"fixture", "choices":[{"index":0,"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        })))
        .mount(&mock).await;
    let mut config = openhuman_embed::RuntimeConfig::default();
    config.local_ai.runtime_enabled = false;
    config.memory.conversations.enabled = false;
    config.agent.session_dual_write = false;
    config.agent.session_shadow_reads = false;
    config.default_temperature = 0.0;
    let before_boot = rss_kib()?;
    let boot_start = Instant::now();
    let runtime = Runtime::builder()
        .config(config)
        .workspace(Workspace::Ephemeral)
        .backend_url(mock.uri())
        .max_agents(count)
        .build()
        .await?;
    let boot_ms = boot_start.elapsed().as_secs_f64() * 1000.0;
    let baseline = rss_kib()?;
    let action = tempfile::tempdir()?;
    let agents = (0..count)
        .map(|index| {
            runtime.agent(
                AgentSpec::new(format!("linux-worker-{index}"))
                    .provider(Provider::openai_compatible(
                        format!("{}/v1", mock.uri()),
                        "fixture",
                    ))
                    .model("fixture")
                    .access(Access::full())
                    .action_dir(action.path())
                    .definition(
                        AgentDefinitionSpec::new()
                            .tools(ToolScopeSpec::Named(vec!["shell".into()])),
                    ),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let registered = rss_kib()?;
    let cold_start = Instant::now();
    let first = agents[0]
        .turn("Say done.")
        .session("cold-session")
        .send()
        .await?;
    anyhow::ensure!(first.reply == "done", "unexpected cold reply");
    let cold_ms = cold_start.elapsed().as_secs_f64() * 1000.0;
    let mut tasks = tokio::task::JoinSet::new();
    let fleet_start = Instant::now();
    for (index, agent) in agents.iter().cloned().enumerate() {
        tasks.spawn(async move {
            let start = Instant::now();
            let outcome = agent
                .turn("Say done.")
                .session(format!("fleet-session-{index}"))
                .send()
                .await?;
            anyhow::ensure!(outcome.reply == "done", "unexpected fleet reply");
            Ok::<_, anyhow::Error>(start.elapsed().as_secs_f64() * 1000.0)
        });
    }
    let mut latencies = Vec::with_capacity(count);
    while let Some(result) = tasks.join_next().await {
        latencies.push(result??);
    }
    let elapsed_ms = fleet_start.elapsed().as_secs_f64() * 1000.0;
    latencies.sort_by(f64::total_cmp);
    let completed = rss_kib()?;
    let percentile = |p: f64| latencies[((count - 1) as f64 * p).round() as usize];
    println!(
        "{}",
        json!({
            "agents":count,"worker_threads":2,"inference":"loopback-http-mock","tool_scope":["shell"],
            "before_boot_rss_kib":before_boot,"runtime_rss_kib":baseline,"registered_rss_kib":registered,
            "after_turns_rss_kib":completed,"marginal_after_turns_mib":completed.saturating_sub(baseline) as f64 / count as f64 / 1024.0,
            "boot_ms":boot_ms,"cold_turn_ms":cold_ms,"fleet_elapsed_ms":elapsed_ms,
            "turn_p50_ms":percentile(0.5),"turn_p95_ms":percentile(0.95),"turn_p99_ms":percentile(0.99)
        })
    );
    Ok(())
}
