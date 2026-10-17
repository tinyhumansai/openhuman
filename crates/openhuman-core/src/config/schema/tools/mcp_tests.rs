use super::*;

#[test]
fn mcp_client_config_toml_round_trips_exposure_and_identity() {
    let raw = r#"
enabled = true

[client_identity]
name = "custom"
title = "Custom Client"
version = "1.2.3"

[[servers]]
name = "docs"
endpoint = "https://example.com/mcp"
expose = "direct"
direct_tools = ["search"]
"#;
    let cfg: McpClientConfig = toml::from_str(raw).expect("parse");
    assert_eq!(cfg.client_identity.name, "custom");
    assert_eq!(cfg.client_identity.title, "Custom Client");
    assert_eq!(cfg.client_identity.version, "1.2.3");
    assert_eq!(cfg.servers[0].expose, McpToolExposure::Direct);
    assert_eq!(cfg.servers[0].direct_tools, vec!["search".to_string()]);

    let again: McpClientConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
    assert_eq!(again.client_identity, cfg.client_identity);
    assert_eq!(again.servers[0].expose, McpToolExposure::Direct);
}

#[test]
fn mcp_client_identity_defaults_to_openhuman_and_partial_tables_keep_them() {
    let cfg: McpClientConfig = toml::from_str("").unwrap();
    assert_eq!(cfg.client_identity, default_client_identity());
    assert_eq!(cfg.client_identity.name, "openhuman-core");

    let cfg: McpClientConfig = toml::from_str("[client_identity]\nname = \"x\"\n").unwrap();
    assert_eq!(cfg.client_identity.name, "x");
    assert_eq!(cfg.client_identity.title, "OpenHuman Core MCP Client");
    assert_eq!(cfg.client_identity.version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn mcp_tool_exposure_defaults_to_deferred() {
    let cfg: McpServerConfig = toml::from_str("name = \"a\"\nendpoint = \"https://e\"\n").unwrap();
    assert_eq!(cfg.expose, McpToolExposure::Deferred);
}
