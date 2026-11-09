use crate::config::Config;
use crate::doctor::contract::{DoctorCtx, DoctorOptions, DoctorTier, Finding, FindingSeverity};
use crate::doctor::runner;

const PARSE: &str = "config/parse";
const UNKNOWN_KEY: &str = "config/unknown-key";
const LEGACY_KEY: &str = "config/legacy-key";

async fn load_config(contents: &str) -> (tempfile::TempDir, Config) {
    let directory = tempfile::tempdir().expect("temporary config directory");
    let config_path = directory.path().join("config.toml");
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).expect("create workspace");
    std::fs::write(&config_path, contents).expect("write config");
    let config = Config::load_from_config_path(&config_path, &workspace)
        .await
        .expect("load config");
    (directory, config)
}

async fn run_config_check(config: Config, check_id: &str) -> Vec<Finding> {
    runner::run(
        &DoctorCtx::from_config(config),
        DoctorOptions {
            tier: DoctorTier::Static,
            only: vec![check_id.to_owned()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("selected config doctor check runs")
    .findings
    .into_iter()
    .filter(|finding| finding.check_id == check_id)
    .collect()
}

fn finding(findings: &[Finding], severity: FindingSeverity) -> &Finding {
    findings
        .iter()
        .find(|finding| finding.severity == severity)
        .expect("expected finding severity")
}

#[tokio::test]
async fn unknown_keys_warn_with_a_suggestion_without_exposing_values() {
    let (_directory, config) = load_config("[memroy]\nengine = \"private-value\"\n").await;
    let findings = run_config_check(config, UNKNOWN_KEY).await;

    let warning = finding(&findings, FindingSeverity::Warn);
    assert_eq!(warning.config_path.as_deref(), Some("memroy"));
    assert!(warning.message.contains("memory"), "{}", warning.message);
    assert!(!warning.message.contains("private-value"));
}

#[tokio::test]
async fn typed_parse_error_reports_field_path_and_backup_fallback_safely() {
    let directory = tempfile::tempdir().expect("temporary config directory");
    let config_path = directory.path().join("config.toml");
    let backup_path = directory.path().join("config.toml.bak");
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).expect("create workspace");
    std::fs::write(&config_path, "[memory]\nengine = 837465\n").expect("write invalid config");
    std::fs::write(&backup_path, "[memory]\nengine = \"backup-engine\"\n")
        .expect("write valid backup");
    let config = Config::load_from_config_path(&config_path, &workspace)
        .await
        .expect("load config from backup");

    let findings = run_config_check(config, PARSE).await;

    let error = finding(&findings, FindingSeverity::Error);
    assert_eq!(error.config_path.as_deref(), Some("memory.engine"));
    assert!(
        error.message.to_ascii_lowercase().contains("backup"),
        "{}",
        error.message
    );
    assert!(!error.message.contains("837465"));
    assert!(!error
        .message
        .contains(&directory.path().display().to_string()));
}

#[tokio::test]
async fn legacy_keys_report_the_actual_source_key_as_info() {
    let (_directory, config) =
        load_config("api_url = \"https://example.test/v1/chat/completions\"\n").await;
    let findings = run_config_check(config, LEGACY_KEY).await;

    let info = finding(&findings, FindingSeverity::Info);
    assert_eq!(info.config_path.as_deref(), Some("api_url"));
    assert!(info.message.contains("inference_url"), "{}", info.message);
}

#[tokio::test]
async fn clean_default_config_passes_each_config_check() {
    for check_id in [PARSE, UNKNOWN_KEY, LEGACY_KEY] {
        let findings = run_config_check(Config::default(), check_id).await;
        assert_eq!(findings.len(), 1, "{check_id}");
        assert_eq!(findings[0].severity, FindingSeverity::Pass, "{check_id}");
    }
}
