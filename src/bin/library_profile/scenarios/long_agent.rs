//! `long-agent`: steady-state many-turn loop on ONE warmed agent — models a
//! long-running opencompany agent. Builds the agent and runs one warm-up turn
//! BEFORE the measured region, then runs N sequential turns inside it, pushing
//! a per-turn checkpoint so the plateau/leak curve is visible.

use anyhow::Result;
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;

use crate::harness::{fixture, measure, ProfileResult};
use crate::mock::PlainTextMock;

const DEFAULT_TURNS: usize = 25;

const PROMPTS: &[&str] = &[
    "Summarise today's Phoenix migration standup in one line.",
    "What is the current staging p99 latency and error rate?",
    "Who owns the rollback runbook and on-call coordination?",
    "When does the phoenix_v2_enabled flag ramp, and what gates it?",
    "Draft a one-sentence status update for the billing-ledger team.",
];

pub async fn run() -> Result<ProfileResult> {
    let turns = std::env::var("OPENHUMAN_PROFILE_TURNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_TURNS);

    let fixture = fixture()?;
    let _ = init_global(256);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let mock = PlainTextMock::new("Phoenix migration is healthy; no action needed.");
    let _provider = test_provider_override::install_model(mock.clone());

    let mut agent = Agent::from_config_for_agent(&fixture.config, "subconscious")?;
    eprintln!("[library-profile] long-agent: warming agent with one pre-measure turn");
    let warm = agent.run_single("Warm-up: confirm you are ready.").await?;
    anyhow::ensure!(!warm.trim().is_empty(), "empty warm-up reply");

    measure("long-agent", turns, Some(turns), move |rec| async move {
        for i in 0..turns {
            let prompt = PROMPTS[i % PROMPTS.len()];
            let reply = agent.run_single(prompt).await?;
            anyhow::ensure!(!reply.trim().is_empty(), "empty reply on turn {i}");
            rec.checkpoint(format!("turn-{i}"))?;
        }
        Ok(())
    })
    .await
}
