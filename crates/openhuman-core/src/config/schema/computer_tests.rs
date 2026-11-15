use super::*;

#[test]
fn defaults_to_jev_and_module_models() {
    let config = ComputerConfig::default();
    assert_eq!(config.decision_model, DecisionModel::Jev);
    assert!(!config.sage_fast);
    assert_eq!(config.rescue_model, None);
    assert_eq!(config.max_rescues, None);
}

#[test]
fn parses_every_decision_model_and_rescue_settings() {
    for (raw, model) in [
        ("jev", DecisionModel::Jev),
        ("open_jev", DecisionModel::OpenJev),
        ("sage", DecisionModel::Sage),
    ] {
        let config: ComputerConfig = toml::from_str(&format!(
            "decision_model = \"{raw}\"\nrescue_model = \"openai/gpt-6-luna\"\nmax_rescues = 0"
        ))
        .unwrap();
        assert_eq!(config.decision_model, model);
        assert_eq!(model.as_str(), raw);
        assert_eq!(config.rescue_model.as_deref(), Some("openai/gpt-6-luna"));
        assert_eq!(config.max_rescues, Some(0));
    }
    assert!(toml::from_str::<ComputerConfig>("decision_model = \"jet\"").is_err());
}

#[test]
fn the_captcha_provider_defaults_to_absent_and_round_trips() {
    assert_eq!(ComputerConfig::default().captcha_provider, None);
    let config: ComputerConfig =
        toml::from_str("captcha_solver = true\ncaptcha_provider = \"surplus\"").unwrap();
    assert_eq!(config.captcha_provider.as_deref(), Some("surplus"));
    let saved = toml::to_string(&config).unwrap();
    assert!(saved.contains("captcha_provider = \"surplus\""), "{saved}");
    let reloaded: ComputerConfig = toml::from_str(&saved).unwrap();
    assert_eq!(reloaded, config);
}
