use super::*;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    // Apply-mode migrations create unified-memory entries. The memory
    // engine's host seams are explicit, so install the test wiring before
    // constructing a configuration that can exercise that path.
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
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

// Apply writes into the bound memory driver, which only exists when a
// memory module is compiled in. With `--no-default-features`,
// `binding::module_provider` substitutes the null provider, and
// `target_memory_backend` then refuses the import rather than reporting
// entries it discarded — correct behaviour, and the opposite of what this
// case asserts. Gated rather than made tolerant of both answers: a test
// that passes on either outcome would stop witnessing the import at all.
// The gates-off half of the same seam is
// `apply_refuses_and_names_the_build_when_no_memory_module_is_compiled_in`
// below.
#[cfg(feature = "modules")]
#[tokio::test]
async fn migrate_openclaw_apply_imports_markdown_entries_into_target_workspace() {
    // Regression for #1440: prior to this PR the Apply path
    // (`dry_run = false`) bailed at `create_memory_for_migration`
    // because the unified namespace memory core hard-disabled it.
    // With the disable removed, Apply must actually move markdown
    // entries from the OpenClaw source workspace into the target.
    // The apply path does real memory work, so it needs the embedding host
    // seam installed. In the default build another test installs the
    // process-global host first; under `--no-default-features` those tests
    // are gated out, so this test must install it itself (idempotent).
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

/// With no memory module compiled in, apply must refuse — and the refusal
/// must name the build, not the user's config.
///
/// `admit` is pure config and never sees the feature flag, so it admits the
/// configured `tinycortex`; `module_provider` then substitutes the null
/// provider because there is no module to bind. The binding that comes back
/// therefore reports `class = Null` with `driver_id = "tinymemory"` and no
/// `fallback` — nothing refused, so there is nothing to fall back from.
///
/// Keyed on the class alone, that landed in the configured-null arm and told
/// a user whose `config.toml` says `driver = "tinycortex"` that memory was
/// "disabled by configuration ([subsystems.memory] driver = \"null\")",
/// pointing them at a line that says the opposite. This asserts both halves:
/// the import is still refused (silent data loss stays impossible), and the
/// reason given is the missing module.
#[cfg(not(feature = "modules"))]
#[tokio::test]
async fn apply_refuses_and_names_the_build_when_no_memory_module_is_compiled_in() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let source = tmp.path().join("openclaw-src");
    std::fs::create_dir_all(&source).unwrap();
    let entry = source.join("MEMORY.md");
    let original = "# Note\nwould be lost";
    std::fs::write(&entry, original).unwrap();

    let err = migrate_openclaw(&config, Some(source), false)
        .await
        .expect_err("apply must refuse with no memory module compiled in");

    assert!(
        err.contains("this build has no memory module compiled in"),
        "refusal must name the missing module as the cause; got: {err}"
    );
    assert!(
        !err.contains("[subsystems.memory] driver = \"null\""),
        "refusal must not blame a config line the user did not write; got: {err}"
    );
    assert!(
        err.contains("the source workspace is untouched"),
        "refusal must still promise the source survived; got: {err}"
    );
    // The promise, checked rather than taken on trust: a refusal that moved
    // or truncated the source would be the very loss it claims to prevent.
    assert_eq!(
        std::fs::read_to_string(&entry).expect("source entry must still be readable"),
        original,
        "the refused import must leave the source workspace byte-identical"
    );
}

/// A driver deliberately given `class = "null"` is not a modules-off build.
///
/// Codex raised this against the first version of the fix above, and it was
/// right. `admit` skips its `built_in_class` check for an id that is not
/// built in, so `[subsystems.memory] driver = "mynull"` with
/// `class = "null"` is admitted verbatim: `class = Null`, no `fallback`, and
/// a `driver_id` that is not `"null"`. Keyed on the id, that landed in the
/// modules-off arm and told a user with a perfectly good modules build that
/// their `modules` feature was off.
///
/// Keying on what `admit` *answered* separates them: here it answers `Null`,
/// where the modules-off case answers `Module` and is then bound to the null
/// provider. This runs in **every** feature configuration, because the
/// confusion it guards against is one a modules-enabled build can hit.
#[tokio::test]
async fn apply_names_a_deliberately_null_classed_driver_rather_than_the_build() {
    use crate::openhuman::config::schema::{MemoryDriverConfig, MemorySubsystemConfig};

    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    let mut drivers = std::collections::BTreeMap::new();
    drivers.insert(
        "mynull".to_string(),
        MemoryDriverConfig {
            class: Some("null".to_string()),
            ..Default::default()
        },
    );
    config.subsystems.memory = MemorySubsystemConfig {
        driver: "mynull".to_string(),
        drivers,
        ..Default::default()
    };

    let source = tmp.path().join("openclaw-src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("MEMORY.md"), "# Note\nkeep me").unwrap();

    let err = migrate_openclaw(&config, Some(source), false)
        .await
        .expect_err("apply must refuse a null-classed driver");

    assert!(
        err.contains("mynull") && err.contains("class"),
        "refusal must name the driver and its configured class; got: {err}"
    );
    assert!(
        !err.contains("no memory module compiled in"),
        "a deliberately null-classed driver must not be reported as a \
         modules-off build; got: {err}"
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

// Apply writes into the bound memory driver, which only exists when a
// memory module is compiled in. With `--no-default-features`,
// `binding::module_provider` substitutes the null provider, and
// `target_memory_backend` then refuses the import rather than reporting
// entries it discarded — correct behaviour, and the opposite of what this
// case asserts. Gated rather than made tolerant of both answers: a test
// that passes on either outcome would stop witnessing the import at all.
// The gates-off half of the same seam is
// `apply_refuses_and_names_the_build_when_no_memory_module_is_compiled_in`
// below.
#[cfg(feature = "modules")]
#[tokio::test]
async fn migrate_hermes_apply_imports_markdown_entries() {
    // Apply does real memory work; install the embedding host seam so this
    // test stands on its own under `--no-default-features` (idempotent).
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
