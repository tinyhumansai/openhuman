use super::*;

async fn registry_names_for(cfg: &Config, tmp: &TempDir) -> Vec<String> {
    let security = Arc::new(SecurityPolicy::default());
    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditSink::disabled(),
        &BrowserConfig::default(),
        &crate::config::HttpRequestConfig::default(),
        tmp.path(),
        cfg,
    )
    .await;
    tool_names(&tools)
}

#[tokio::test]
async fn composio_tools_register_in_direct_mode_without_an_app_session() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = test_config(&tmp);
    cfg.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.into();
    cfg.composio.api_key = Some("ck_direct_test".into());

    let names = registry_names_for(&cfg, &tmp).await;
    assert_contains_all(
        &names,
        &[
            "composio_list_toolkits",
            "composio_list_connections",
            "composio_authorize",
            "composio_list_tools",
            "composio_execute",
            "composio_connect",
        ],
    );
}

#[tokio::test]
async fn composio_tools_stay_unregistered_in_backend_mode_without_an_app_session() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);

    let names = registry_names_for(&cfg, &tmp).await;
    assert!(
        !names.iter().any(|n| n.starts_with("composio_")),
        "backend mode without a session must not register composio tools; got: {names:?}"
    );
}

#[tokio::test]
async fn composio_tools_register_in_backend_mode_once_a_host_registers_the_route() {
    crate::integrations::composio::test_backend_route::register();
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    store_test_session_token(&cfg).await;

    let names = registry_names_for(&cfg, &tmp).await;
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
