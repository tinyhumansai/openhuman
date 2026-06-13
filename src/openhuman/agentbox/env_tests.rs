use super::env::{collect_gmi_config, GmiConfig};

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
