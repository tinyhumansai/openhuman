use anyhow::Result;

use crate::openhuman::config::Config;

/// Deterministically run queued memory-tree jobs until no immediately
/// claimable work remains. Intended for tests that need the async pipeline
/// to settle without spawning background tasks.
pub async fn drain_until_idle(config: &Config) -> Result<()> {
    loop {
        if !super::worker::run_once(config).await? {
            break;
        }
    }
    Ok(())
}
