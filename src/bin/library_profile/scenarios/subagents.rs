//! `subagents`: run one real orchestrator chat turn that spawns two real
//! researcher subagents through the parallel-delegation tool.

use anyhow::Result;
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::config::schema::SubconsciousMode;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;
use openhuman_core::openhuman::subconscious::LongLivedSession;

use crate::harness::{fixture, measure, ProfileResult};
use crate::mock::{subagent_marker, SubagentMock};

pub async fn run() -> Result<ProfileResult> {
    let fixture = fixture()?;
    let _ = init_global(256);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let mock = SubagentMock::new();
    let _provider = test_provider_override::install_model(mock.clone());
    if std::env::var_os("OPENHUMAN_PROFILE_PREWARM_SUBAGENTS").is_some() {
        eprintln!("[library-profile] subagents: prewarming one full turn");
        let warmup = LongLivedSession::with_thread(
            fixture.config.workspace_dir.clone(),
            SubconsciousMode::Aggressive,
            "profile:warmup".into(),
        );
        let outcome = warmup
            .process_promoted("Please research the Phoenix migration.", false)
            .await
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(!outcome.response.is_empty(), "empty warmup response");
        mock.prompts.lock().expect("mock prompt lock").clear();
    }
    let session = LongLivedSession::with_thread(
        fixture.config.workspace_dir.clone(),
        SubconsciousMode::Aggressive,
        "profile:orchestrator".into(),
    );
    measure("subagents", 2, None, |_rec| async {
        let outcome = session
            .process_promoted("Please research the Phoenix migration.", false)
            .await
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(!outcome.response.is_empty(), "empty orchestrator response");
        let prompts = mock.prompts.lock().expect("mock prompt lock");
        anyhow::ensure!(prompts.iter().any(|p| p.contains(&subagent_marker(1))));
        anyhow::ensure!(prompts.iter().any(|p| p.contains(&subagent_marker(2))));
        Ok(())
    })
    .await
}
