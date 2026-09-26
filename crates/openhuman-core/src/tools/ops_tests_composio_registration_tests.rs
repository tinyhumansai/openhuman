use super::*;

fn registry_names_for(cfg: &Config, tmp: &TempDir) -> Vec<String> {
    let security = Arc::new(SecurityPolicy::default());
    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
        &BrowserConfig::default(),
        &crate::config::HttpRequestConfig::default(),
        tmp.path(),
        &HashMap::new(),
        cfg,
    );
    tool_names(&tools)
}

#[test]
fn composio_tools_register_in_direct_mode_without_an_app_session() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = test_config(&tmp);
    cfg.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.into();
    cfg.composio.api_key = Some("ck_direct_test".into());
    assert!(crate::integrations::build_client(&cfg).is_none());

    let names = registry_names_for(&cfg, &tmp);
    assert_contains_all(
        &names,
        &[
            "composio_list_toolkits",
            "composio_list_connections",
            "composio_authorize",
            "composio_list_tools",
            "composio_execute",
        ],
    );
}

#[test]
fn composio_tools_stay_unregistered_in_backend_mode_without_an_app_session() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    assert!(crate::integrations::build_client(&cfg).is_none());

    let names = registry_names_for(&cfg, &tmp);
    assert!(
        !names.iter().any(|n| n.starts_with("composio_")),
        "backend mode without a session must not register composio tools; got: {names:?}"
    );
}
