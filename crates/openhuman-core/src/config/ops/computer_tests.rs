use super::*;

#[test]
fn patch_selects_decision_model_and_rescue_settings() {
    let next = patched(
        &ComputerConfig::default(),
        ComputerSettingsPatch {
            decision_model: Some("OpenJev".into()),
            rescue_model: Some(" openai/gpt-6-luna ".into()),
            planner_model: Some("anthropic/claude-sonnet-5".into()),
            max_rescues: Some(0),
            sage_fast: Some(true),
            ..ComputerSettingsPatch::default()
        },
    )
    .unwrap();
    assert_eq!(next.decision_model, DecisionModel::OpenJev);
    assert_eq!(next.rescue_model.as_deref(), Some("openai/gpt-6-luna"));
    assert_eq!(
        next.planner_model.as_deref(),
        Some("anthropic/claude-sonnet-5")
    );
    assert_eq!(next.max_rescues, Some(0));
    assert!(next.sage_fast);
}

#[test]
fn empty_model_restores_the_module_default() {
    let mut current = ComputerConfig::default();
    current.rescue_model = Some("x/y".into());
    let next = patched(
        &current,
        ComputerSettingsPatch {
            rescue_model: Some("  ".into()),
            ..ComputerSettingsPatch::default()
        },
    )
    .unwrap();
    assert_eq!(next.rescue_model, None);
}

#[test]
fn invalid_values_are_refused() {
    let base = ComputerConfig::default();
    for patch in [
        ComputerSettingsPatch {
            decision_model: Some("jet".into()),
            ..ComputerSettingsPatch::default()
        },
        ComputerSettingsPatch {
            max_rescues: Some(6),
            ..ComputerSettingsPatch::default()
        },
        ComputerSettingsPatch {
            rescue_model: Some("has space".into()),
            ..ComputerSettingsPatch::default()
        },
        ComputerSettingsPatch {
            max_captcha_attempts: Some(0),
            ..ComputerSettingsPatch::default()
        },
        ComputerSettingsPatch {
            max_captcha_attempts: Some(MAX_CAPTCHA_ATTEMPTS + 1),
            ..ComputerSettingsPatch::default()
        },
        ComputerSettingsPatch {
            captcha_model: Some("jev preview".into()),
            ..ComputerSettingsPatch::default()
        },
    ] {
        assert!(patched(&base, patch).is_err());
    }
}

#[tokio::test]
async fn apply_saves_the_section() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.workspace_dir = dir.path().join("workspace");
    apply_computer_settings(
        &mut config,
        ComputerSettingsPatch {
            decision_model: Some("sage".into()),
            ..ComputerSettingsPatch::default()
        },
    )
    .await
    .unwrap();
    let saved = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(saved.contains("[computer]"), "{saved}");
    assert!(saved.contains("decision_model = \"sage\""), "{saved}");
}

#[test]
fn patch_turns_the_captcha_solver_on_with_its_model_and_attempts() {
    let next = patched(
        &ComputerConfig::default(),
        ComputerSettingsPatch {
            captcha_solver: Some(true),
            captcha_model: Some(" jev-1.14 ".into()),
            max_captcha_attempts: Some(4),
            ..ComputerSettingsPatch::default()
        },
    )
    .unwrap();
    assert!(next.captcha_solver);
    assert_eq!(next.captcha_model.as_deref(), Some("jev-1.14"));
    assert_eq!(next.max_captcha_attempts, Some(4));
    assert!(!ComputerConfig::default().captcha_solver, "off by default");
}
