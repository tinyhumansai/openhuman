//! `subconscious`: one promoted subconscious turn WITHOUT delegation. Same
//! `LongLivedSession` path as `subagents`, but the mock returns a direct text
//! response (no `spawn_parallel_agents` tool call). Complements `subagents`.

use anyhow::Result;
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::config::schema::SubconsciousMode;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;
use openhuman_core::openhuman::subconscious::LongLivedSession;

use crate::harness::{fixture, measure, ProfileResult};
use crate::mock::PlainTextMock;

pub async fn run() -> Result<ProfileResult> {
    let fixture = fixture()?;
    let _ = init_global(256);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let mock = PlainTextMock::new("Phoenix migration is on track; nothing needs your attention.");
    let _provider = test_provider_override::install_model(mock.clone());

    if std::env::var_os("OPENHUMAN_PROFILE_PREWARM_SUBAGENTS").is_some() {
        eprintln!("[library-profile] subconscious: prewarming one full turn");
        let warmup = LongLivedSession::with_thread(
            fixture.config.workspace_dir.clone(),
            SubconsciousMode::Aggressive,
            "profile:warmup".into(),
        );
        let outcome = warmup
            .process_promoted("Please review the Phoenix migration.", false)
            .await
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(!outcome.response.is_empty(), "empty warmup response");
        mock.prompts.lock().expect("mock prompt lock").clear();
    }

    let session = LongLivedSession::with_thread(
        fixture.config.workspace_dir.clone(),
        SubconsciousMode::Aggressive,
        "profile:subconscious".into(),
    );
    measure("subconscious", 1, None, |_rec| async {
        let outcome = session
            .process_promoted("Please review the Phoenix migration.", false)
            .await
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(!outcome.response.is_empty(), "empty subconscious response");
        Ok(())
    })
    .await
}
