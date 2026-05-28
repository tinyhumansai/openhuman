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

pub async fn migrate_hermes(
    config: &Config,
    source_workspace: Option<PathBuf>,
    dry_run: bool,
) -> Result<RpcOutcome<MigrationReport>, String> {
    let report = migration::migrate_hermes_memory(config, source_workspace, dry_run)
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
    async fn migrate_openclaw_apply_imports_markdown_entries_into_target_workspace() {
        // Regression for #1440: prior to this PR the Apply path
        // (`dry_run = false`) bailed at `create_memory_for_migration`
        // because the unified namespace memory core hard-disabled it.
        // With the disable removed, Apply must actually move markdown
        // entries from the OpenClaw source workspace into the target.
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        // Fake OpenClaw workspace with two markdown entries — no
        // brain.db needed; the migration path reads MEMORY.md + any
        // memory/*.md files.
        let source = tmp.path().join("openclaw-src");
        std::fs::create_dir_all(source.join("memory")).unwrap();
        std::fs::write(source.join("MEMORY.md"), "# Top-level note\nimport me").unwrap();
        std::fs::write(
            source.join("memory").join("sprint.md"),
            "# Sprint plan\nweek one design",
        )
        .unwrap();

        let outcome = migrate_openclaw(&config, Some(source), false)
            .await
            .expect("apply path should succeed on the unified core after #1440");
        let report = outcome.value;
        assert!(!report.dry_run, "apply must produce a non-dry-run report");
        assert!(
            report.stats.imported >= 1,
            "apply must import at least one entry; stats={:?}",
            report.stats
        );
    }

    #[tokio::test]
    async fn migrate_openclaw_returns_error_for_missing_source_workspace() {
        // Pointing at a non-existent source directory must surface as
        // an Err from the wrapper (the underlying `migrate_openclaw_memory`
        // bails with "OpenClaw workspace not found at ..."), so the
        // JSON-RPC adapter can return the error to the caller.
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let missing = tmp.path().join("does-not-exist").join("nested");
        let err = migrate_openclaw(&config, Some(missing), false)
            .await
            .expect_err("missing source workspace must surface as Err");
        assert!(
            !err.is_empty(),
            "error string must be non-empty so the RPC caller sees a reason"
        );
    }

    // ── Hermes migration tests ──────────────────────────────────────

    #[tokio::test]
    async fn migrate_hermes_dry_run_on_empty_source_returns_report() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let result = migrate_hermes(&config, Some(tmp.path().to_path_buf()), true).await;
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
    async fn migrate_hermes_apply_imports_markdown_entries() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        let source = tmp.path().join("hermes-src");
        std::fs::create_dir_all(&source).unwrap();
        // Cover the full Hermes file mapping (MEMORY.md / USER.md / SOUL.md)
        // so the apply path is exercised for every category — including the
        // `Custom("persona")` SOUL.md branch which @graycyrus called out as
        // untested in the original review on this PR.
        std::fs::write(source.join("MEMORY.md"), "# Agent memory\nremember this").unwrap();
        std::fs::write(
            source.join("USER.md"),
            "# User profile\nprefers concise answers",
        )
        .unwrap();
        std::fs::write(source.join("SOUL.md"), "# Persona\ncalm and curious").unwrap();

        let outcome = migrate_hermes(&config, Some(source), false)
            .await
            .expect("apply path should succeed");
        let report = outcome.value;
        assert!(!report.dry_run);
        assert!(
            report.stats.imported >= 3,
            "apply must import all 3 entries; stats={:?}",
            report.stats
        );
        assert_eq!(report.stats.from_markdown, 3);
    }

    #[tokio::test]
    async fn migrate_hermes_skips_missing_optional_files() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        let source = tmp.path().join("hermes-src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("MEMORY.md"), "# Memory\nonly memory").unwrap();

        let outcome = migrate_hermes(&config, Some(source), true)
            .await
            .expect("should succeed with partial files");
        let report = outcome.value;
        assert_eq!(report.stats.from_markdown, 1);
        assert!(
            report.warnings.iter().any(|w| w.contains("USER.md")),
            "warnings should mention missing USER.md: {:?}",
            report.warnings
        );
        assert!(
            report.warnings.iter().any(|w| w.contains("SOUL.md")),
            "warnings should mention missing SOUL.md: {:?}",
            report.warnings
        );
    }

    #[tokio::test]
    async fn migrate_hermes_returns_error_for_missing_source() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let missing = tmp.path().join("does-not-exist");
        let err = migrate_hermes(&config, Some(missing), false)
            .await
            .expect_err("missing source must surface as Err");
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn migrate_hermes_refuses_self_migration() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        let err = migrate_hermes(&config, Some(config.workspace_dir.clone()), false)
            .await
            .expect_err("self-migration must be refused");
        assert!(
            err.contains("self-migration"),
            "error should mention self-migration: {err}"
        );
    }
}
