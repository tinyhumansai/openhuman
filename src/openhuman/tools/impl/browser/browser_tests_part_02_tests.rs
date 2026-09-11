use super::*;

// ── is_supported_browser_action ───────────────────────────────────────────

#[test]
fn supported_action_detection_is_exhaustive() {
    let supported = [
        "open",
        "snapshot",
        "click",
        "fill",
        "type",
        "get_text",
        "get_title",
        "get_url",
        "wait",
        "press",
        "hover",
        "scroll",
        "is_visible",
        "close",
        "find",
        "mouse_move",
        "mouse_click",
        "mouse_drag",
        "key_type",
        "key_press",
    ];
    for action in supported {
        assert!(
            is_supported_browser_action(action),
            "expected '{action}' to be supported"
        );
    }
    assert!(!is_supported_browser_action("teleport"));
    assert!(!is_supported_browser_action("screenshot"));
    assert!(!is_supported_browser_action("screen_capture"));
    assert!(!is_supported_browser_action(""));
}

// ── BrowserBackendKind::as_str ────────────────────────────────────────────

#[test]
fn browser_backend_kind_as_str_roundtrips() {
    assert_eq!(BrowserBackendKind::AgentBrowser.as_str(), "agent_browser");
    assert_eq!(BrowserBackendKind::Playwright.as_str(), "playwright");
    assert_eq!(BrowserBackendKind::RustNative.as_str(), "rust_native");
    assert_eq!(BrowserBackendKind::ComputerUse.as_str(), "computer_use");
    assert_eq!(BrowserBackendKind::Auto.as_str(), "auto");
}

// ── validate_computer_use_action ──────────────────────────────────────────

#[test]
fn validate_computer_use_action_open_requires_url() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["*".into()],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig::default(),
    );
    let params = serde_json::Map::new(); // missing url
    assert!(tool.validate_computer_use_action("open", &params).is_err());

    // Valid url
    let mut valid_params = serde_json::Map::new();
    valid_params.insert("url".into(), json!("https://example.com"));
    // validate_url will reject example.com as not in allowlist unless we use * — but we
    // are using "*" so should pass.
    assert!(tool
        .validate_computer_use_action("open", &valid_params)
        .is_ok());
}

#[test]
fn validate_computer_use_action_mouse_requires_xy() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["*".into()],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig::default(),
    );
    // missing both x and y
    let empty = serde_json::Map::new();
    assert!(tool
        .validate_computer_use_action("mouse_move", &empty)
        .is_err());

    // valid
    let mut valid = serde_json::Map::new();
    valid.insert("x".into(), json!(100_i64));
    valid.insert("y".into(), json!(200_i64));
    assert!(tool
        .validate_computer_use_action("mouse_move", &valid)
        .is_ok());
    assert!(tool
        .validate_computer_use_action("mouse_click", &valid)
        .is_ok());
}

#[test]
fn validate_computer_use_action_drag_requires_all_coords() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["*".into()],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig::default(),
    );
    let partial = {
        let mut m = serde_json::Map::new();
        m.insert("from_x".into(), json!(10_i64));
        m.insert("from_y".into(), json!(20_i64));
        // missing to_x and to_y
        m
    };
    assert!(tool
        .validate_computer_use_action("mouse_drag", &partial)
        .is_err());

    let full = {
        let mut m = serde_json::Map::new();
        m.insert("from_x".into(), json!(10_i64));
        m.insert("from_y".into(), json!(20_i64));
        m.insert("to_x".into(), json!(100_i64));
        m.insert("to_y".into(), json!(200_i64));
        m
    };
    assert!(tool
        .validate_computer_use_action("mouse_drag", &full)
        .is_ok());
}

#[test]
fn validate_computer_use_action_unknown_action_passes() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["*".into()],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig::default(),
    );
    // unknown actions should pass validation (no-op match arm)
    let empty = serde_json::Map::new();
    assert!(tool
        .validate_computer_use_action("key_type", &empty)
        .is_ok());
}

// ── coordinate validation edge cases ──────────────────────────────────────

#[test]
fn validate_coordinate_negative_limit_errors() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    assert!(tool.validate_coordinate("x", 5, Some(-1)).is_err());
}

#[test]
fn validate_coordinate_no_limit_allows_any_non_negative() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    assert!(tool.validate_coordinate("x", 99999, None).is_ok());
    assert!(tool.validate_coordinate("x", 0, None).is_ok());
}

// ── backend_name ──────────────────────────────────────────────────────────

#[test]
fn backend_name_covers_all_variants() {
    assert_eq!(backend_name(ResolvedBackend::AgentBrowser), "agent_browser");
    assert_eq!(backend_name(ResolvedBackend::Playwright), "playwright");
    assert_eq!(backend_name(ResolvedBackend::RustNative), "rust_native");
    assert_eq!(backend_name(ResolvedBackend::ComputerUse), "computer_use");
}

// ── ComputerUseConfig Debug (redacts api_key) ─────────────────────────────

#[test]
fn computer_use_config_debug_redacts_api_key() {
    let cfg = ComputerUseConfig {
        api_key: Some("supersecret".into()),
        ..ComputerUseConfig::default()
    };
    let dbg = format!("{cfg:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains("supersecret"));
}

#[test]
fn computer_use_config_debug_none_api_key() {
    let cfg = ComputerUseConfig::default();
    let dbg = format!("{cfg:?}");
    assert!(dbg.contains("None"));
}

// ── computer_use endpoint validation ─────────────────────────────────────

#[test]
fn computer_use_endpoint_rejects_empty_endpoint() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec![],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig {
            endpoint: String::new(),
            ..ComputerUseConfig::default()
        },
    );
    assert!(tool.computer_use_endpoint_url().is_err());
}

#[test]
fn computer_use_endpoint_rejects_zero_timeout() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec![],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig {
            timeout_ms: 0,
            ..ComputerUseConfig::default()
        },
    );
    assert!(tool.computer_use_endpoint_url().is_err());
}

#[test]
fn computer_use_endpoint_rejects_non_http_scheme() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec![],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig {
            endpoint: "ftp://127.0.0.1:21/actions".into(),
            ..ComputerUseConfig::default()
        },
    );
    assert!(tool.computer_use_endpoint_url().is_err());
}

#[test]
fn computer_use_endpoint_accepts_local_http() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec![],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig {
            endpoint: "http://127.0.0.1:8787/v1/actions".into(),
            ..ComputerUseConfig::default()
        },
    );
    assert!(tool.computer_use_endpoint_url().is_ok());
}

// ── browser tool Tool trait metadata ─────────────────────────────────────

#[test]
fn browser_tool_description_non_empty() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    assert!(!tool.description().is_empty());
    assert!(tool.description().contains("browser"));
}

#[test]
fn browser_tool_schema_has_required_action() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("action")));
    let actions = schema["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum");
    assert!(actions.contains(&json!("snapshot")));
    assert!(!actions.contains(&json!("screenshot")));
    assert!(!actions.contains(&json!("screen_capture")));
    assert!(schema["properties"].get("full_page").is_none());
    assert!(schema["properties"].get("path").is_none());
}

#[test]
fn browser_tool_spec_roundtrip() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    let spec = tool.spec();
    assert_eq!(spec.name, "browser");
    assert!(spec.parameters.is_object());
}
