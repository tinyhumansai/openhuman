use super::env::{agentbox_mode_enabled, collect_gmi_config, GmiConfig, AGENTBOX_MODE_ENV_VAR};

#[test]
fn collect_returns_some_when_all_three_vars_present() {
    let cfg = collect_gmi_config(|k| match k {
        "GMI_MAAS_BASE_URL" => Some("https://api.gmi-serving.com".into()),
        "GMI_MAAS_API_KEY" => Some("sk-test".into()),
        "GMI_MODELS" => Some("deepseek-ai/DeepSeek-V4-Pro".into()),
        _ => None,
    });
    assert_eq!(
        cfg,
        Ok(GmiConfig {
            base_url: "https://api.gmi-serving.com".into(),
            api_key: "sk-test".into(),
            model: "deepseek-ai/DeepSeek-V4-Pro".into(),
        })
    );
}

#[test]
fn collect_reports_each_missing_var() {
    let cfg = collect_gmi_config(|k| match k {
        "GMI_MAAS_BASE_URL" => Some("u".into()),
        _ => None,
    });
    let err = cfg.unwrap_err();
    assert!(err.contains("GMI_MAAS_API_KEY"), "missing api key reported");
    assert!(err.contains("GMI_MODELS"), "missing model reported");
    assert!(
        !err.contains("GMI_MAAS_BASE_URL"),
        "present var not reported missing"
    );
}

// `OPENHUMAN_AGENTBOX_MODE` is process-global, so hold the shared AgentBox env
// lock while toggling it and restore the prior value before releasing the lock.
#[test]
fn mode_enabled_only_when_flag_is_exactly_one() {
    let _lock = super::test_support::test_env_lock();
    let prior = std::env::var(AGENTBOX_MODE_ENV_VAR).ok();

    std::env::set_var(AGENTBOX_MODE_ENV_VAR, "1");
    assert!(agentbox_mode_enabled(), "exactly \"1\" enables the mode");

    std::env::set_var(AGENTBOX_MODE_ENV_VAR, "0");
    assert!(!agentbox_mode_enabled(), "\"0\" does not enable the mode");

    std::env::set_var(AGENTBOX_MODE_ENV_VAR, "true");
    assert!(
        !agentbox_mode_enabled(),
        "non-\"1\" truthy values do not enable the mode"
    );

    std::env::remove_var(AGENTBOX_MODE_ENV_VAR);
    assert!(
        !agentbox_mode_enabled(),
        "unset means disabled (desktop default)"
    );

    match prior {
        Some(v) => std::env::set_var(AGENTBOX_MODE_ENV_VAR, v),
        None => std::env::remove_var(AGENTBOX_MODE_ENV_VAR),
    }
}

#[test]
fn collect_treats_blank_string_as_missing() {
    let cfg = collect_gmi_config(|k| match k {
        "GMI_MAAS_BASE_URL" => Some("".into()),
        "GMI_MAAS_API_KEY" => Some("sk".into()),
        "GMI_MODELS" => Some("m".into()),
        _ => None,
    });
    assert!(cfg.is_err());
}

#[test]
fn collect_trims_leading_and_trailing_whitespace() {
    let cfg = collect_gmi_config(|k| match k {
        "GMI_MAAS_BASE_URL" => Some("  https://api.gmi-serving.com\n".into()),
        "GMI_MAAS_API_KEY" => Some(" sk-test ".into()),
        "GMI_MODELS" => Some("\tdeepseek-ai/DeepSeek-V4-Pro\t".into()),
        _ => None,
    });
    assert_eq!(
        cfg,
        Ok(GmiConfig {
            base_url: "https://api.gmi-serving.com".into(),
            api_key: "sk-test".into(),
            model: "deepseek-ai/DeepSeek-V4-Pro".into(),
        })
    );
}
