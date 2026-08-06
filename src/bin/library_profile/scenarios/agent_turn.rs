//! `agent-turn`: the minimal "embed OpenHuman as a library" unit — a single
//! cold agent turn built directly from config, with a plain-text mock provider
//! (no tool calls, no delegation).

use anyhow::Result;
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;

use crate::harness::{fixture, measure, ProfileResult};
use crate::mock::PlainTextMock;

pub async fn run() -> Result<ProfileResult> {
    let fixture = fixture()?;
    let _ = init_global(256);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let mock = PlainTextMock::new("The Phoenix migration is healthy and on track.");
    let _provider = test_provider_override::install_model(mock.clone());
    eprintln!("[library-profile] agent-turn: registries ready, mock installed");

    measure("agent-turn", 1, None, |_rec| async {
        let mut agent = Agent::from_config_for_agent(&fixture.config, "subconscious")?;
        let reply = agent
            .run_single("Give me a one-line status on the Phoenix migration.")
            .await?;
        anyhow::ensure!(!reply.trim().is_empty(), "empty agent reply");
        Ok(())
    })
    .await
}
