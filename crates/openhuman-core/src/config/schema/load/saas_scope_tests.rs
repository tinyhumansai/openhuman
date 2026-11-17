use super::*;

#[test]
fn single_user_mode_keeps_process_resolution() {
    assert!(resolve(false, None).is_none());
    assert!(resolve(false, Some(Config::default())).is_none());
}

#[test]
fn saas_mode_returns_the_scoped_config() {
    let scoped = Config {
        workspace_dir: "/srv/oh/agents/u-1/workspace".into(),
        ..Config::default()
    };
    let loaded = resolve(true, Some(scoped)).unwrap().unwrap();
    assert_eq!(
        loaded.workspace_dir,
        std::path::PathBuf::from("/srv/oh/agents/u-1/workspace")
    );
}

#[test]
fn saas_mode_without_a_scope_fails_instead_of_guessing() {
    let err = resolve(true, None).unwrap().unwrap_err();
    assert!(err.to_string().contains("no config in scope"), "{err}");
}
