use super::*;

#[test]
fn summary_reflects_presets_and_knobs() {
    let summary = RuntimeBuilder::desktop()
        .listen("127.0.0.1", 7790)
        .action_dir("/srv/work")
        .token(TokenSource::Fixed(std::sync::Arc::new(
            "bearer".to_string(),
        )))
        .summary();
    assert_eq!(summary.host_kind, HostKind::TauriShell);
    assert!(matches!(summary.workspace, Workspace::Inherit));
    assert_eq!(summary.config_source, ConfigSource::Discovered);
    assert_eq!(summary.domains, Some(DomainSet::full()));
    assert_eq!(summary.services, Some(ServiceSet::desktop()));
    assert!(summary.fixed_token);
    assert_eq!(summary.listen_host.as_deref(), Some("127.0.0.1"));
    assert_eq!(summary.listen_port, Some(7790));
    assert_eq!(summary.action_dir, Some(PathBuf::from("/srv/work")));
    assert!(summary.controller_extensions.is_empty());
    assert!(summary.tool_ranker.is_none());
}

#[test]
fn summary_hides_secrets() {
    let summary = RuntimeBuilder::new().api_key("th_secret_key").summary();
    assert!(summary.has_api_key);
    assert!(!format!("{summary:?}").contains("th_secret_key"));
}
