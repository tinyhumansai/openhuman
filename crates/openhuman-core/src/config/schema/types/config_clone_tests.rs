use super::*;
use std::path::PathBuf;

#[test]
fn clone_copies_every_serialized_field() {
    let mut config = Config::default();
    config.workspace_dir = PathBuf::from("/tmp/clone-ws");
    config.default_model = Some("clone-model".into());
    config.chat_onboarding_completed = true;
    config.audit.max_size_mb = 7;

    let cloned = config.clone();

    assert_eq!(cloned.workspace_dir, config.workspace_dir);
    assert_eq!(cloned.default_model, config.default_model);
    assert_eq!(cloned.audit.max_size_mb, 7);
    assert!(cloned.chat_onboarding_completed);
    assert_eq!(
        serde_json::to_value(&cloned).expect("cloned config serializes"),
        serde_json::to_value(&config).expect("config serializes"),
    );
}

#[test]
fn clone_is_independent_of_the_original() {
    let config = Config {
        default_model: Some("before".into()),
        ..Config::default()
    };
    let mut cloned = config.clone();
    cloned.default_model = Some("after".into());
    assert_eq!(config.default_model.as_deref(), Some("before"));
}
