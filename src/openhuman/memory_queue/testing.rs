//! Test helpers for the jobs runtime — not used in production code paths.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    #[tokio::test]
    async fn drain_until_idle_is_noop_when_queue_is_empty() {
        let (_tmp, cfg) = test_config();
        drain_until_idle(&cfg).await.unwrap();
    }
}
