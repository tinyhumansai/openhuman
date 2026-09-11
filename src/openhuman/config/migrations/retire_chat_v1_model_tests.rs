use super::*;
use crate::openhuman::config::Config;

#[test]
fn leaves_chat_v1_default_model_unchanged() {
    let mut config = Config::default();
    config.default_model = Some("chat-v1".to_string());

    let stats = run(&mut config).expect("migration should succeed");

    assert!(!stats.default_model_remapped);
    assert_eq!(
        config.default_model.as_deref(),
        Some("chat-v1"),
        "chat-v1 is the canonical chat tier and must not be remapped"
    );
}

#[test]
fn leaves_other_model_values_unchanged() {
    let mut config = Config::default();
    config.default_model = Some("reasoning-v1".to_string());

    let stats = run(&mut config).expect("migration should succeed");

    assert!(!stats.default_model_remapped);
    assert_eq!(config.default_model.as_deref(), Some("reasoning-v1"));
}

#[test]
fn leaves_none_default_model_unchanged() {
    let mut config = Config::default();
    config.default_model = None;

    let stats = run(&mut config).expect("migration should succeed");

    assert!(!stats.default_model_remapped);
    assert_eq!(config.default_model, None);
}

#[test]
fn idempotent_when_already_reasoning_quick_v1() {
    let mut config = Config::default();
    config.default_model = Some("reasoning-quick-v1".to_string());

    let stats = run(&mut config).expect("migration should succeed");

    assert!(!stats.default_model_remapped);
    assert_eq!(config.default_model.as_deref(), Some("reasoning-quick-v1"));
}
