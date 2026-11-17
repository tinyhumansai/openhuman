use crate::config::schema::load::{DotenvLoadProvenance, ProvenanceSource};
use crate::config::Config;

use super::contract::{
    DoctorCtx, DoctorOptions, DoctorTier, Finding, FindingSeverity, ValueSource,
};
use super::runner;

const ENV_OVERRIDE: &str = "config/env-override";
const CONFIG_SOURCE: &str = "config/source";

fn config_with_file_env_override() -> Config {
    let mut config = Config::default();
    config.default_model = Some("resolved-model-secret".to_owned());
    config.config_path = "/diagnostics/config.toml".into();
    let document = toml::from_str::<toml::Value>(r#"default_model = "file-model-secret""#)
        .expect("test config document parses");
    config.load_provenance_mut().record_file_keys(&document);
    config.load_provenance_mut().record_env_override(
        "default_model",
        "OPENHUMAN_DEFAULT_MODEL",
        ProvenanceSource::Env,
    );
    config
}

fn selected_findings<'a>(findings: &'a [Finding], check_id: &str) -> Vec<&'a Finding> {
    findings
        .iter()
        .filter(|finding| finding.check_id == check_id)
        .collect()
}

async fn run_config_check(config: Config, check_id: &str) -> Vec<Finding> {
    run_context_check(DoctorCtx::from_config(config), check_id)
        .await
        .findings
}

async fn run_context_check(ctx: DoctorCtx, check_id: &str) -> crate::doctor::DoctorReport {
    runner::run(
        &ctx,
        DoctorOptions {
            tier: DoctorTier::Static,
            only: vec![check_id.to_owned()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("selected config doctor check runs")
}

#[tokio::test]
async fn environment_override_warns_when_the_file_explicitly_sets_the_field() {
    let findings = run_config_check(config_with_file_env_override(), ENV_OVERRIDE).await;
    let matching = selected_findings(&findings, ENV_OVERRIDE);

    assert_eq!(
        matching.len(),
        1,
        "one finding identifies the overridden field"
    );
    assert_eq!(matching[0].severity, FindingSeverity::Warn);
    assert_eq!(matching[0].config_path.as_deref(), Some("default_model"));
    assert_eq!(matching[0].value_source, Some(ValueSource::Env));
    assert!(matching[0].message.contains("OPENHUMAN_DEFAULT_MODEL"));
    let serialized = serde_json::to_string(&matching).expect("finding serializes");
    assert!(!serialized.contains("resolved-model-secret"));
    assert!(!serialized.contains("file-model-secret"));
}

#[test]
fn context_value_source_maps_logical_paths_without_labeling_filesystem_paths() {
    let mut config = Config::default();
    let document = toml::from_str::<toml::Value>(r#"default_model = "file-model-secret""#)
        .expect("test config document parses");
    config.load_provenance_mut().record_file_keys(&document);
    config
        .load_provenance_mut()
        .record_cli_override("chat_provider");
    config.load_provenance_mut().record_env_override(
        "modules.overrides[].source",
        "OPENHUMAN_MODULE_SOURCE",
        ProvenanceSource::Dotenv,
    );
    let ctx = DoctorCtx::from_config(config);

    assert_eq!(
        ctx.value_source_for("modules.overrides[2].source"),
        Some(ValueSource::Dotenv)
    );
    assert_eq!(
        ctx.value_source_for("default_model"),
        Some(ValueSource::File)
    );
    assert_eq!(
        ctx.value_source_for("chat_provider"),
        Some(ValueSource::Cli)
    );
    assert_eq!(
        ctx.value_source_for("output_language"),
        Some(ValueSource::Default)
    );
    assert_eq!(ctx.value_source_for("/var/lib/openhuman/config.toml"), None);
    assert_eq!(ctx.value_source_for("config.toml"), None);
}

#[tokio::test]
async fn config_source_reports_the_active_user_directory_resolution_source() {
    let ctx = DoctorCtx::from_config(Config::default()).with_profile_metadata(
        Some(crate::config::schema::profile_layout::LayoutVersion::V2),
        Some("active_user.toml".to_owned()),
    );
    let report = run_context_check(ctx, CONFIG_SOURCE).await;
    let matching = selected_findings(&report.findings, CONFIG_SOURCE);

    assert_eq!(
        report.config_resolution_source.as_deref(),
        Some("active_user.toml")
    );
    assert_eq!(report.profile_layout_version, Some(2));
    assert!(matching.iter().any(|finding| {
        finding
            .message
            .contains("config_resolution_source: active_user.toml")
            && finding.message.contains("profile_layout_version: 2")
    }));
}

#[tokio::test]
async fn environment_override_is_info_when_the_file_does_not_set_the_field() {
    let mut config = Config::default();
    config.load_provenance_mut().record_env_override(
        "agent.tool_dispatcher",
        "OPENHUMAN_TOOL_DISPATCHER",
        ProvenanceSource::Env,
    );

    let findings = run_config_check(config, ENV_OVERRIDE).await;
    let matching = selected_findings(&findings, ENV_OVERRIDE);

    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].severity, FindingSeverity::Info);
    assert_eq!(
        matching[0].config_path.as_deref(),
        Some("agent.tool_dispatcher")
    );
    assert!(matching[0].message.contains("OPENHUMAN_TOOL_DISPATCHER"));
}

#[tokio::test]
async fn environment_override_check_enumerates_multiple_unrelated_fields() {
    let mut config = config_with_file_env_override();
    config.load_provenance_mut().record_env_override(
        "search.enabled",
        "OPENHUMAN_SEARCH_ENABLED",
        ProvenanceSource::Env,
    );

    let findings = run_config_check(config, ENV_OVERRIDE).await;
    let matching = selected_findings(&findings, ENV_OVERRIDE);
    let by_path = matching
        .iter()
        .map(|finding| {
            (
                finding
                    .config_path
                    .as_deref()
                    .expect("field path is reported"),
                finding.severity,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(matching.len(), 2);
    assert_eq!(by_path.get("default_model"), Some(&FindingSeverity::Warn));
    assert_eq!(by_path.get("search.enabled"), Some(&FindingSeverity::Info));
}

#[tokio::test]
async fn dotenv_shadowing_and_paths_are_reported_without_values() {
    let mut config = config_with_file_env_override();
    config
        .load_provenance_mut()
        .set_dotenv(Some(DotenvLoadProvenance::from_variables(
            "/diagnostics/.env".into(),
            ["OPENHUMAN_DEFAULT_MODEL", "OPENHUMAN_OUTPUT_LANGUAGE"],
            ["OPENHUMAN_DEFAULT_MODEL"],
        )));

    let findings = run_config_check(config, CONFIG_SOURCE).await;
    let matching = selected_findings(&findings, CONFIG_SOURCE);

    assert!(
        !matching.is_empty(),
        "config/source describes the active configuration sources"
    );
    let serialized = serde_json::to_string(&matching).expect("finding serializes");
    assert!(serialized.contains("/diagnostics/config.toml"));
    assert!(serialized.contains("/diagnostics/.env"));
    assert!(serialized.contains("OPENHUMAN_DEFAULT_MODEL"));
    assert!(!serialized.contains("resolved-model-secret"));
    assert!(!serialized.contains("file-model-secret"));
}

#[tokio::test]
async fn config_source_reports_active_cli_model_and_provider_overrides() {
    let mut config = Config::default();
    config.config_path = "/diagnostics/config.toml".into();
    config.default_model = Some("private-model-secret".to_owned());
    config.chat_provider = Some("private-provider-secret".to_owned());
    {
        let provenance = config.load_provenance_mut();
        provenance.record_cli_override("default_model");
        provenance.record_cli_override("chat_provider");
    }

    let findings = run_config_check(config, CONFIG_SOURCE).await;
    let matching = selected_findings(&findings, CONFIG_SOURCE);

    assert!(
        !matching.is_empty(),
        "config/source describes the active configuration sources"
    );
    let serialized = serde_json::to_string(&matching).expect("finding serializes");
    assert!(serialized.contains("--model"));
    assert!(serialized.contains("--provider"));
    assert!(!serialized.contains("private-model-secret"));
    assert!(!serialized.contains("private-provider-secret"));
}
