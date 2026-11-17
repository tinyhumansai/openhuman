use super::*;
use serde_json::json;

fn expected_proxy_config() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["get", "set", "disable", "list_services", "apply_env", "clear_env"],
                "default": "get"
            },
            "enabled": {
                "type": "boolean",
                "description": "Enable or disable proxy"
            },
            "scope": {
                "type": "string",
                "description": "Proxy scope: environment | openhuman | services"
            },
            "http_proxy": {
                "type": ["string", "null"],
                "description": "HTTP proxy URL"
            },
            "https_proxy": {
                "type": ["string", "null"],
                "description": "HTTPS proxy URL"
            },
            "all_proxy": {
                "type": ["string", "null"],
                "description": "Fallback proxy URL for all protocols"
            },
            "no_proxy": {
                "description": "Comma-separated string or array of NO_PROXY entries",
                "oneOf": [
                    {"type": "string"},
                    {"type": "array", "items": {"type": "string"}}
                ]
            },
            "services": {
                "description": "Comma-separated string or array of service selectors used when scope=services",
                "oneOf": [
                    {"type": "string"},
                    {"type": "array", "items": {"type": "string"}}
                ]
            },
            "clear_env": {
                "type": "boolean",
                "description": "When action=disable, clear process proxy environment variables"
            }
        }
    })
}

#[test]
fn proxy_config_static_schema_matches_json_literal() {
    let tool = ProxyConfigTool::new(
        std::sync::Arc::new(crate::config::Config::default()),
        std::sync::Arc::new(crate::security::SecurityPolicy::default()),
    );
    assert_eq!(
        tinytools::Tool::parameters_schema(&tool),
        expected_proxy_config()
    );
}
