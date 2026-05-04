use super::*;

static BROWSER_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn normalize_domains_works() {
    let domains = vec![
        "  Example.COM  ".into(),
        "docs.example.com".into(),
        String::new(),
    ];
    let normalized = normalize_domains(domains);
    assert_eq!(normalized, vec!["example.com", "docs.example.com"]);
}

#[test]
fn extract_host_works() {
    assert_eq!(
        extract_host("https://example.com/path").unwrap(),
        "example.com"
    );
    assert_eq!(
        extract_host("https://Sub.Example.COM:8080/").unwrap(),
        "sub.example.com"
    );
}

#[test]
fn extract_host_handles_ipv6() {
    // IPv6 with brackets (required for URLs with ports)
    assert_eq!(extract_host("https://[::1]/path").unwrap(), "[::1]");
    // IPv6 with brackets and port
    assert_eq!(
        extract_host("https://[2001:db8::1]:8080/path").unwrap(),
        "[2001:db8::1]"
    );
    // IPv6 with brackets, trailing slash
    assert_eq!(extract_host("https://[fe80::1]/").unwrap(), "[fe80::1]");
}

#[test]
fn is_private_host_detects_local() {
    assert!(is_private_host("localhost"));
    assert!(is_private_host("app.localhost"));
    assert!(is_private_host("printer.local"));
    assert!(is_private_host("127.0.0.1"));
    assert!(is_private_host("192.168.1.1"));
    assert!(is_private_host("10.0.0.1"));
    assert!(!is_private_host("example.com"));
    assert!(!is_private_host("google.com"));
}

#[test]
fn is_private_host_blocks_multicast_and_reserved() {
    assert!(is_private_host("224.0.0.1")); // multicast
    assert!(is_private_host("255.255.255.255")); // broadcast
    assert!(is_private_host("100.64.0.1")); // shared address space
    assert!(is_private_host("240.0.0.1")); // reserved
    assert!(is_private_host("192.0.2.1")); // documentation
    assert!(is_private_host("198.51.100.1")); // documentation
    assert!(is_private_host("203.0.113.1")); // documentation
    assert!(is_private_host("198.18.0.1")); // benchmarking
}

#[test]
fn is_private_host_catches_ipv6() {
    assert!(is_private_host("::1"));
    assert!(is_private_host("[::1]"));
    assert!(is_private_host("0.0.0.0"));
}

#[test]
fn is_private_host_catches_mapped_ipv4() {
    // IPv4-mapped IPv6 addresses
    assert!(is_private_host("::ffff:127.0.0.1"));
    assert!(is_private_host("::ffff:10.0.0.1"));
    assert!(is_private_host("::ffff:192.168.1.1"));
}

#[test]
fn is_private_host_catches_ipv6_private_ranges() {
    // Unique-local (fc00::/7)
    assert!(is_private_host("fd00::1"));
    assert!(is_private_host("fc00::1"));
    // Link-local (fe80::/10)
    assert!(is_private_host("fe80::1"));
    // Public IPv6 should pass
    assert!(!is_private_host("2001:db8::1"));
}

#[test]
fn validate_url_blocks_ipv6_ssrf() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec!["*".into()], None);
    assert!(tool.validate_url("https://[::1]/").is_err());
    assert!(tool.validate_url("https://[::ffff:127.0.0.1]/").is_err());
    assert!(tool
        .validate_url("https://[::ffff:10.0.0.1]:8080/")
        .is_err());
}

#[test]
fn host_matches_allowlist_exact() {
    let allowed = vec!["example.com".into()];
    assert!(host_matches_allowlist("example.com", &allowed));
    assert!(host_matches_allowlist("sub.example.com", &allowed));
    assert!(!host_matches_allowlist("notexample.com", &allowed));
}

#[test]
fn host_matches_allowlist_wildcard() {
    let allowed = vec!["*.example.com".into()];
    assert!(host_matches_allowlist("sub.example.com", &allowed));
    assert!(host_matches_allowlist("example.com", &allowed));
    assert!(!host_matches_allowlist("other.com", &allowed));
}

#[test]
fn host_matches_allowlist_star() {
    let allowed = vec!["*".into()];
    assert!(host_matches_allowlist("anything.com", &allowed));
    assert!(host_matches_allowlist("example.org", &allowed));
}

#[test]
fn browser_backend_parser_accepts_supported_values() {
    assert_eq!(
        BrowserBackendKind::parse("agent_browser").unwrap(),
        BrowserBackendKind::AgentBrowser
    );
    assert_eq!(
        BrowserBackendKind::parse("rust-native").unwrap(),
        BrowserBackendKind::RustNative
    );
    assert_eq!(
        BrowserBackendKind::parse("computer_use").unwrap(),
        BrowserBackendKind::ComputerUse
    );
    assert_eq!(
        BrowserBackendKind::parse("auto").unwrap(),
        BrowserBackendKind::Auto
    );
}

#[test]
fn browser_backend_parser_rejects_unknown_values() {
    assert!(BrowserBackendKind::parse("playwright").is_err());
}

#[test]
fn browser_tool_default_backend_is_agent_browser() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec!["example.com".into()], None);
    assert_eq!(
        tool.configured_backend().unwrap(),
        BrowserBackendKind::AgentBrowser
    );
}

#[test]
fn browser_tool_accepts_auto_backend_config() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["example.com".into()],
        None,
        "auto".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig::default(),
    );
    assert_eq!(tool.configured_backend().unwrap(), BrowserBackendKind::Auto);
}

#[test]
fn browser_tool_accepts_computer_use_backend_config() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["example.com".into()],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig::default(),
    );
    assert_eq!(
        tool.configured_backend().unwrap(),
        BrowserBackendKind::ComputerUse
    );
}

#[test]
fn computer_use_endpoint_rejects_public_http_by_default() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["example.com".into()],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig {
            endpoint: "http://computer-use.example.com/v1/actions".into(),
            ..ComputerUseConfig::default()
        },
    );

    assert!(tool.computer_use_endpoint_url().is_err());
}

#[test]
fn computer_use_endpoint_requires_https_for_public_remote() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["example.com".into()],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig {
            endpoint: "https://computer-use.example.com/v1/actions".into(),
            allow_remote_endpoint: true,
            ..ComputerUseConfig::default()
        },
    );

    assert!(tool.computer_use_endpoint_url().is_ok());
}

#[test]
fn computer_use_coordinate_validation_applies_limits() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["example.com".into()],
        None,
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig {
            max_coordinate_x: Some(100),
            max_coordinate_y: Some(100),
            ..ComputerUseConfig::default()
        },
    );

    assert!(tool
        .validate_coordinate("x", 50, tool.computer_use.max_coordinate_x)
        .is_ok());
    assert!(tool
        .validate_coordinate("x", 101, tool.computer_use.max_coordinate_x)
        .is_err());
    assert!(tool
        .validate_coordinate("y", -1, tool.computer_use.max_coordinate_y)
        .is_err());
}

#[test]
fn browser_tool_name() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec!["example.com".into()], None);
    assert_eq!(tool.name(), "browser");
}

#[test]
fn browser_tool_validates_url() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec!["example.com".into()], None);

    // Valid
    assert!(tool.validate_url("https://example.com").is_ok());
    assert!(tool.validate_url("https://sub.example.com/path").is_ok());

    // Invalid - not in allowlist
    assert!(tool.validate_url("https://other.com").is_err());

    // Invalid - private host
    assert!(tool.validate_url("https://localhost").is_err());
    assert!(tool.validate_url("https://127.0.0.1").is_err());

    // Invalid - not https
    assert!(tool.validate_url("ftp://example.com").is_err());

    // file:// URLs blocked (local file exfiltration risk)
    assert!(tool.validate_url("file:///tmp/test.html").is_err());
}

#[test]
fn browser_tool_empty_allowlist_blocks() {
    let _guard = BROWSER_ENV_LOCK.lock().expect("env lock poisoned");
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
    assert!(tool.validate_url("https://example.com").is_err());
}

#[test]
fn browser_tool_empty_allowlist_allows_with_env_flag() {
    let _guard = BROWSER_ENV_LOCK.lock().expect("env lock poisoned");
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL", "1");
    assert!(tool.validate_url("https://example.com").is_ok());
    std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
}

#[test]
fn computer_use_only_action_detection_is_correct() {
    assert!(is_computer_use_only_action("mouse_move"));
    assert!(is_computer_use_only_action("mouse_click"));
    assert!(is_computer_use_only_action("mouse_drag"));
    assert!(is_computer_use_only_action("key_type"));
    assert!(is_computer_use_only_action("key_press"));
    assert!(is_computer_use_only_action("screen_capture"));
    assert!(!is_computer_use_only_action("open"));
    assert!(!is_computer_use_only_action("snapshot"));
}

#[test]
fn unavailable_action_error_preserves_backend_context() {
    assert_eq!(
        unavailable_action_for_backend_error("mouse_move", ResolvedBackend::AgentBrowser),
        "Action 'mouse_move' is unavailable for backend 'agent_browser'"
    );
    assert_eq!(
        unavailable_action_for_backend_error("mouse_move", ResolvedBackend::RustNative),
        "Action 'mouse_move' is unavailable for backend 'rust_native'"
    );
}

// ── parse_browser_action ───────────────────────────────────────────────

#[test]
fn parse_open_requires_url() {
    assert!(parse_browser_action("open", &json!({})).is_err());
    let action = parse_browser_action("open", &json!({"url": "https://example.com"})).unwrap();
    assert!(matches!(action, BrowserAction::Open { url } if url == "https://example.com"));
}

#[test]
fn parse_snapshot_defaults() {
    let action = parse_browser_action("snapshot", &json!({})).unwrap();
    if let BrowserAction::Snapshot {
        interactive_only,
        compact,
        depth,
    } = action
    {
        // Both default to true
        assert!(interactive_only);
        assert!(compact);
        assert!(depth.is_none());
    } else {
        panic!("expected Snapshot");
    }
}

#[test]
fn parse_snapshot_with_depth() {
    let action = parse_browser_action(
        "snapshot",
        &json!({"depth": 3, "interactive_only": false, "compact": false}),
    )
    .unwrap();
    if let BrowserAction::Snapshot {
        interactive_only,
        compact,
        depth,
    } = action
    {
        assert!(!interactive_only);
        assert!(!compact);
        assert_eq!(depth, Some(3));
    } else {
        panic!("expected Snapshot");
    }
}

#[test]
fn parse_click_requires_selector() {
    assert!(parse_browser_action("click", &json!({})).is_err());
    let action = parse_browser_action("click", &json!({"selector": "@e1"})).unwrap();
    assert!(matches!(action, BrowserAction::Click { selector } if selector == "@e1"));
}

#[test]
fn parse_fill_requires_selector_and_value() {
    assert!(parse_browser_action("fill", &json!({"selector": "#id"})).is_err());
    assert!(parse_browser_action("fill", &json!({"value": "hello"})).is_err());
    let action =
        parse_browser_action("fill", &json!({"selector": "#id", "value": "hello"})).unwrap();
    assert!(
        matches!(action, BrowserAction::Fill { selector, value } if selector == "#id" && value == "hello")
    );
}

#[test]
fn parse_type_requires_selector_and_text() {
    assert!(parse_browser_action("type", &json!({"selector": "#id"})).is_err());
    assert!(parse_browser_action("type", &json!({"text": "hello"})).is_err());
    let action =
        parse_browser_action("type", &json!({"selector": "#id", "text": "hello"})).unwrap();
    assert!(
        matches!(action, BrowserAction::Type { selector, text } if selector == "#id" && text == "hello")
    );
}

#[test]
fn parse_get_text_requires_selector() {
    assert!(parse_browser_action("get_text", &json!({})).is_err());
    let action = parse_browser_action("get_text", &json!({"selector": "h1"})).unwrap();
    assert!(matches!(action, BrowserAction::GetText { selector } if selector == "h1"));
}

#[test]
fn parse_get_title_and_get_url() {
    assert!(matches!(
        parse_browser_action("get_title", &json!({})).unwrap(),
        BrowserAction::GetTitle
    ));
    assert!(matches!(
        parse_browser_action("get_url", &json!({})).unwrap(),
        BrowserAction::GetUrl
    ));
}

#[test]
fn parse_screenshot_optional_fields() {
    let action = parse_browser_action("screenshot", &json!({})).unwrap();
    if let BrowserAction::Screenshot { path, full_page } = action {
        assert!(path.is_none());
        assert!(!full_page);
    } else {
        panic!("expected Screenshot");
    }

    let action2 = parse_browser_action(
        "screenshot",
        &json!({"path": "/tmp/s.png", "full_page": true}),
    )
    .unwrap();
    if let BrowserAction::Screenshot { path, full_page } = action2 {
        assert_eq!(path.as_deref(), Some("/tmp/s.png"));
        assert!(full_page);
    }
}

#[test]
fn parse_wait_optional_fields() {
    let action = parse_browser_action("wait", &json!({"selector": "#el"})).unwrap();
    if let BrowserAction::Wait { selector, ms, text } = action {
        assert_eq!(selector.as_deref(), Some("#el"));
        assert!(ms.is_none());
        assert!(text.is_none());
    }

    let action2 = parse_browser_action("wait", &json!({"ms": 500})).unwrap();
    if let BrowserAction::Wait { selector, ms, text } = action2 {
        assert!(selector.is_none());
        assert_eq!(ms, Some(500));
        assert!(text.is_none());
    }
}

#[test]
fn parse_press_requires_key() {
    assert!(parse_browser_action("press", &json!({})).is_err());
    let action = parse_browser_action("press", &json!({"key": "Enter"})).unwrap();
    assert!(matches!(action, BrowserAction::Press { key } if key == "Enter"));
}

#[test]
fn parse_hover_requires_selector() {
    assert!(parse_browser_action("hover", &json!({})).is_err());
    let action = parse_browser_action("hover", &json!({"selector": "button"})).unwrap();
    assert!(matches!(action, BrowserAction::Hover { selector } if selector == "button"));
}

#[test]
fn parse_scroll_requires_direction() {
    assert!(parse_browser_action("scroll", &json!({})).is_err());
    let action = parse_browser_action("scroll", &json!({"direction": "down"})).unwrap();
    if let BrowserAction::Scroll { direction, pixels } = action {
        assert_eq!(direction, "down");
        assert!(pixels.is_none());
    }

    let action2 =
        parse_browser_action("scroll", &json!({"direction": "up", "pixels": 100})).unwrap();
    if let BrowserAction::Scroll { direction, pixels } = action2 {
        assert_eq!(direction, "up");
        assert_eq!(pixels, Some(100));
    }
}

#[test]
fn parse_is_visible_requires_selector() {
    assert!(parse_browser_action("is_visible", &json!({})).is_err());
    let action = parse_browser_action("is_visible", &json!({"selector": ".btn"})).unwrap();
    assert!(matches!(action, BrowserAction::IsVisible { selector } if selector == ".btn"));
}

#[test]
fn parse_close_no_args() {
    assert!(matches!(
        parse_browser_action("close", &json!({})).unwrap(),
        BrowserAction::Close
    ));
}

#[test]
fn parse_find_requires_by_value_action() {
    assert!(parse_browser_action("find", &json!({"value": "v", "find_action": "click"})).is_err());
    assert!(parse_browser_action("find", &json!({"by": "role", "find_action": "click"})).is_err());
    assert!(parse_browser_action("find", &json!({"by": "role", "value": "v"})).is_err());

    let action = parse_browser_action(
        "find",
        &json!({"by": "role", "value": "button", "find_action": "click"}),
    )
    .unwrap();
    if let BrowserAction::Find {
        by,
        value,
        action,
        fill_value,
    } = action
    {
        assert_eq!(by, "role");
        assert_eq!(value, "button");
        assert_eq!(action, "click");
        assert!(fill_value.is_none());
    }
}

#[test]
fn parse_find_with_fill_value() {
    let action = parse_browser_action(
        "find",
        &json!({
            "by": "label",
            "value": "Email",
            "find_action": "fill",
            "fill_value": "user@example.com"
        }),
    )
    .unwrap();
    if let BrowserAction::Find { fill_value, .. } = action {
        assert_eq!(fill_value.as_deref(), Some("user@example.com"));
    }
}

#[test]
fn parse_unsupported_action_errors() {
    assert!(parse_browser_action("teleport", &json!({})).is_err());
    assert!(parse_browser_action("", &json!({})).is_err());
}

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
        "screenshot",
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
        "screen_capture",
    ];
    for action in supported {
        assert!(
            is_supported_browser_action(action),
            "expected '{action}' to be supported"
        );
    }
    assert!(!is_supported_browser_action("teleport"));
    assert!(!is_supported_browser_action(""));
}

// ── BrowserBackendKind::as_str ────────────────────────────────────────────

#[test]
fn browser_backend_kind_as_str_roundtrips() {
    assert_eq!(BrowserBackendKind::AgentBrowser.as_str(), "agent_browser");
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
}

#[test]
fn browser_tool_spec_roundtrip() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    let spec = tool.spec();
    assert_eq!(spec.name, "browser");
    assert!(spec.parameters.is_object());
}
