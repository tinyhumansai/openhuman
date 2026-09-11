use super::*;

#[test]
fn names_are_stable() {
    let cfg = Arc::new(Config::default());
    assert_eq!(
        McpSetupSearchTool::new(cfg.clone()).name(),
        "mcp_setup_search"
    );
    assert_eq!(McpSetupGetTool::new(cfg.clone()).name(), "mcp_setup_get");
    assert_eq!(
        McpSetupRequestSecretTool::new(cfg.clone()).name(),
        "mcp_setup_request_secret"
    );
    assert_eq!(
        McpSetupTestConnectionTool::new(cfg.clone()).name(),
        "mcp_setup_test_connection"
    );
    assert_eq!(
        McpSetupInstallAndConnectTool::new(cfg).name(),
        "mcp_setup_install_and_connect"
    );
}

#[test]
fn read_str_map_rejects_non_string_values() {
    let args = json!({ "env_refs": { "K": 42 } });
    assert!(read_str_map(&args, "env_refs").is_err());
}
