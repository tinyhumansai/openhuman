use crate::doctor::{DoctorCtx, DoctorOptions, DoctorTier, FindingSeverity};

const ID: &str = "config/runtime-settings";

async fn run_check(config: crate::config::Config) -> Vec<crate::doctor::Finding> {
    crate::doctor::runner::run(
        &DoctorCtx::from_config(config),
        DoctorOptions {
            tier: DoctorTier::Static,
            only: vec![ID.to_owned()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("runtime config check is registered")
    .findings
}

#[tokio::test]
async fn former_runtime_settings_findings_keep_their_paths_without_exposing_values() {
    use crate::config::{DelegateAgentConfig, ModelRouteConfig};

    let mut config = crate::config::Config::default();
    config.reliability.fallback_providers = vec!["private-model-name".to_owned()];
    config.model_routes = vec![
        ModelRouteConfig {
            hint: "private-route-name".to_owned(),
            model: String::new(),
        },
        ModelRouteConfig {
            hint: String::new(),
            model: "private-model-name".to_owned(),
        },
    ];
    let delegate = DelegateAgentConfig {
        model: String::new(),
        system_prompt: None,
        temperature: None,
        max_depth: 3,
    };
    config
        .agents
        .insert("private-agent-name".to_owned(), delegate);

    let findings = run_check(config).await;
    let paths = findings
        .iter()
        .filter(|finding| finding.check_id == ID)
        .filter_map(|finding| finding.config_path.as_deref())
        .collect::<Vec<_>>();

    for path in [
        "reliability.fallback_providers",
        "model_routes[0].model",
        "model_routes[1].hint",
        "agents[0].model",
    ] {
        assert!(
            paths.contains(&path),
            "missing {path} finding: {findings:#?}"
        );
    }
    assert!(findings
        .iter()
        .filter(|finding| finding.check_id == ID)
        .all(|finding| finding.severity == FindingSeverity::Warn
            && !finding.fix_hint.trim().is_empty()));
    assert!(findings.iter().all(|finding| {
        !finding.message.contains("private-model-name")
            && !finding.message.contains("private-route-name")
            && !finding.message.contains("private-agent-name")
    }));
    assert!(!findings
        .iter()
        .any(|finding| finding.check_id == "doctor/selection"));
}

#[tokio::test]
async fn no_channel_warning_is_emitted_only_when_channels_are_enabled() {
    let mut enabled = crate::config::Config::default();
    enabled.components.channels = true;
    let findings = run_check(enabled).await;
    assert!(findings.iter().any(|finding| {
        finding.check_id == ID
            && finding.config_path.as_deref() == Some("channels_config")
            && finding.severity == FindingSeverity::Warn
    }));

    let mut disabled = crate::config::Config::default();
    disabled.components.channels = false;
    let findings = run_check(disabled).await;
    assert!(!findings.iter().any(|finding| {
        finding.check_id == ID && finding.config_path.as_deref() == Some("channels_config")
    }));
}
