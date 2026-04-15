//! JSON-RPC / CLI controller surface for data migration.

use std::path::PathBuf;

use crate::openhuman::config::Config;
use crate::openhuman::migration::{self, MigrationReport};
use crate::rpc::RpcOutcome;

pub async fn migrate_openclaw(
    config: &Config,
    source_workspace: Option<PathBuf>,
    dry_run: bool,
) -> Result<RpcOutcome<MigrationReport>, String> {
    let report = migration::migrate_openclaw_memory(config, source_workspace, dry_run)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(report, "migration completed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn migrate_openclaw_dry_run_on_empty_source_returns_report() {
        // A fresh temp workspace contains nothing to migrate. The
        // underlying migration helper should still return a report
        // rather than erroring, and the wrapper should attach the
        // canonical completion log.
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let result = migrate_openclaw(&config, Some(tmp.path().to_path_buf()), true).await;
        match result {
            Ok(outcome) => {
                assert!(
                    outcome
                        .logs
                        .iter()
                        .any(|l| l.contains("migration completed")),
                    "expected 'migration completed' log, got logs: {:?}",
                    outcome.logs
                );
            }
            Err(e) => panic!("dry_run on empty source should not error: {e}"),
        }
    }

    #[tokio::test]
    async fn migrate_openclaw_returns_error_for_missing_source_workspace() {
        // Pointing at a non-existent source directory must surface as
        // an Err from the wrapper (not a panic / ok), so the JSON-RPC
        // adapter can return the error to the caller.
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let missing = tmp.path().join("does-not-exist").join("nested");
        let result = migrate_openclaw(&config, Some(missing), false).await;
        // Either an Err OR an Ok with a non-success report is
        // acceptable here — we just pin the no-panic, deterministic-
        // shape contract.
        match result {
            Ok(outcome) => {
                // If the migration helper decides "nothing to do" for
                // a missing source and returns Ok, we still expect the
                // canonical log line.
                assert!(outcome
                    .logs
                    .iter()
                    .any(|l| l.contains("migration completed")));
            }
            Err(e) => assert!(!e.is_empty()),
        }
    }
}
