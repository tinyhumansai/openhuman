use super::*;

#[test]
fn sanitize_error_message_truncates_long_messages() {
    let long_msg = "x".repeat(500);
    let sanitized = sanitize_error_message(&long_msg);
    assert!(
        sanitized.chars().count() <= 243,
        "should be at most 240 chars + '...'"
    );
    assert!(
        sanitized.ends_with("..."),
        "truncated message should end with '...'"
    );
}

#[test]
fn sanitize_error_message_does_not_truncate_short_messages() {
    let short = "Something went wrong";
    let sanitized = sanitize_error_message(short);
    assert_eq!(sanitized, short);
}

#[test]
fn sanitize_error_message_replaces_all_sensitive_variants() {
    // camelCase variants
    let msg = "Error for connectedAccountId and entityId and userId";
    let sanitized = sanitize_error_message(msg);
    assert!(
        !sanitized.contains("connectedAccountId"),
        "camelCase connectedAccountId should be redacted"
    );
    assert!(
        !sanitized.contains("entityId"),
        "camelCase entityId should be redacted"
    );
    assert!(
        !sanitized.contains("userId"),
        "camelCase userId should be redacted"
    );
}

// ── composio_auth_config enabled detection ────────────────────────────────

#[test]
fn auth_config_enabled_by_flag() {
    let cfg = ComposioAuthConfig {
        id: "cfg_x".into(),
        status: None,
        enabled: Some(true),
    };
    assert!(cfg.is_enabled());
}

#[test]
fn auth_config_not_enabled_when_both_missing() {
    let cfg = ComposioAuthConfig {
        id: "cfg_x".into(),
        status: None,
        enabled: None,
    };
    assert!(!cfg.is_enabled());
}

// ── map_v3_tools_to_actions: item without slug falls back to name ─────────

#[test]
fn map_v3_tools_uses_name_when_slug_missing() {
    let items = vec![ComposioV3Tool {
        slug: None,
        name: Some("My Tool".into()),
        description: None,
        app_name: Some("myapp".into()),
        toolkit: None,
        input_parameters: None,
        output_parameters: None,
    }];
    let actions = map_v3_tools_to_actions(items);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].name, "My Tool");
    assert_eq!(actions[0].app_name.as_deref(), Some("myapp"));
}

#[test]
fn map_v3_tools_skips_items_without_slug_or_name() {
    let items = vec![ComposioV3Tool {
        slug: None,
        name: None,
        description: Some("desc".into()),
        app_name: None,
        toolkit: None,
        input_parameters: None,
        output_parameters: None,
    }];
    let actions = map_v3_tools_to_actions(items);
    assert!(
        actions.is_empty(),
        "item with no slug or name should be filtered out"
    );
}

#[test]
fn map_v3_tools_prefers_toolkit_slug_over_app_name() {
    let items = vec![ComposioV3Tool {
        slug: Some("tool-slug".into()),
        name: None,
        description: None,
        app_name: Some("fallback-app".into()),
        toolkit: Some(ComposioToolkitRef {
            slug: Some("preferred-app".into()),
            name: None,
        }),
        input_parameters: None,
        output_parameters: None,
    }];
    let actions = map_v3_tools_to_actions(items);
    assert_eq!(actions[0].app_name.as_deref(), Some("preferred-app"));
}

// ── category ──────────────────────────────────────────────────────────────

#[test]
fn composio_tool_category_is_skill() {
    use crate::openhuman::tools::traits::ToolCategory;
    let tool = ComposioTool::new("key", None, test_security());
    assert_eq!(tool.category(), ToolCategory::Workflow);
}

// ── v3 /connected_accounts shape parsing ───────────────────────────
//
// Two upstream shapes covered:
//   1. `toolkit` as a plain string slug (older payloads)
//   2. `toolkit` as a nested `{ slug, ... }` object (newer payloads,
//      mirroring the `de_string_or_object` drift handled by `types.rs`)
// Plus an `appName` fallback for payloads that omit `toolkit` entirely.

#[test]
fn connected_account_toolkit_slug_from_string() {
    let raw: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "ca_1",
        "status": "ACTIVE",
        "toolkit": "gmail",
        "created_at": "2026-05-15T00:00:00Z"
    }))
    .unwrap();
    assert_eq!(raw.id, "ca_1");
    assert_eq!(raw.status.as_deref(), Some("ACTIVE"));
    assert_eq!(raw.toolkit_slug().as_deref(), Some("gmail"));
    assert_eq!(raw.created_at.as_deref(), Some("2026-05-15T00:00:00Z"));
}

#[test]
fn connected_account_toolkit_slug_from_nested_object() {
    let raw: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "ca_2",
        "status": "INITIATED",
        "toolkit": {"slug": "slack", "logo": "https://example.test/slack.png"}
    }))
    .unwrap();
    assert_eq!(raw.toolkit_slug().as_deref(), Some("slack"));
}

#[test]
fn connected_account_toolkit_slug_fallback_to_app_name() {
    let raw: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "ca_3",
        "status": "ACTIVE",
        "appName": "notion"
    }))
    .unwrap();
    assert_eq!(raw.toolkit_slug().as_deref(), Some("notion"));
}

#[test]
fn connected_account_toolkit_slug_returns_none_when_unrecognized() {
    let raw: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "ca_4",
        "status": "PENDING",
        "toolkit": {"unrelated": 42}
    }))
    .unwrap();
    assert!(raw.toolkit_slug().is_none());
}

#[test]
fn connected_account_tolerates_missing_fields() {
    // All optional fields absent — the row must still parse so a
    // malformed Composio response doesn't blow up `list_connections`.
    let raw: ComposioConnectedAccount = serde_json::from_value(json!({"id": "ca_5"})).unwrap();
    assert_eq!(raw.id, "ca_5");
    assert!(raw.status.is_none());
    assert!(raw.toolkit_slug().is_none());
    assert!(raw.created_at.is_none());
}

#[test]
fn connected_account_accepts_camelcase_created_at() {
    // Tolerate both `created_at` (canonical) and `createdAt` (drift).
    let raw: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "ca_6",
        "createdAt": "2026-05-15T00:00:00Z"
    }))
    .unwrap();
    assert_eq!(raw.created_at.as_deref(), Some("2026-05-15T00:00:00Z"));
}

// ── API key trimming (issue #2323) ────────────────────────
//
// Composio v3 rejects API keys with leading/trailing whitespace as
// "Invalid API key format" (Sentry TAURI-RUST-D3). The constructor must
// strip surrounding whitespace defensively, but MUST preserve internal
// whitespace so legitimate keys containing spaces are not corrupted.

#[test]
fn composio_tool_trims_surrounding_whitespace_in_api_key() {
    let tool = ComposioTool::new(" key123 ", None, test_security());
    assert_eq!(tool.api_key, "key123");
}

#[test]
fn composio_tool_trims_trailing_newline_in_api_key() {
    // The real-world Sentry case: secret store payloads frequently carry a
    // trailing newline (clipboard paste, file read). It must be stripped.
    let tool = ComposioTool::new("key123\n", None, test_security());
    assert_eq!(tool.api_key, "key123");
}

#[test]
fn composio_tool_preserves_internal_whitespace_in_api_key() {
    // Pins the trim-scope: a future refactor must NOT widen this to
    // `replace(' ', "")` or similar — only surrounding whitespace is stripped.
    let tool = ComposioTool::new("k1 k2", None, test_security());
    assert_eq!(tool.api_key, "k1 k2");
}

#[test]
fn composio_tool_accepts_empty_api_key_without_panic() {
    let tool = ComposioTool::new("", None, test_security());
    assert_eq!(tool.api_key, "");
}

#[test]
fn is_loopback_http_url_accepts_real_loopback_hosts() {
    assert!(is_loopback_http_url("http://127.0.0.1:8080/api/v3/tools"));
    assert!(is_loopback_http_url("http://localhost:3000/"));
    assert!(is_loopback_http_url("http://[::1]:9000/tools"));
}

#[test]
fn is_loopback_http_url_rejects_userinfo_smuggling_and_non_loopback() {
    // Prefix-matching would have accepted these; host parsing rejects them.
    assert!(!is_loopback_http_url(
        "http://127.0.0.1:8080@evil.com/api/v3/tools"
    ));
    assert!(!is_loopback_http_url("http://localhost:8080@evil.com/"));
    assert!(!is_loopback_http_url("http://evil.com:8080/"));
    // HTTPS and unparseable inputs are not loopback-HTTP.
    assert!(!is_loopback_http_url("https://127.0.0.1:8080/"));
    assert!(!is_loopback_http_url("not a url"));
}
