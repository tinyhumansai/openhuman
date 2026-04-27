use super::*;
use serde_json::json;

#[test]
fn test_register_and_route() {
    let router = WebhookRouter::new(None);
    router
        .register("uuid-1", "gmail", Some("Gmail Webhook".into()), None)
        .unwrap();

    assert_eq!(router.route("uuid-1"), Some("gmail".to_string()));
    assert_eq!(router.route("uuid-nonexistent"), None);
}

#[test]
fn test_ownership_enforcement() {
    let router = WebhookRouter::new(None);
    router
        .register("uuid-1", "gmail", Some("Gmail".into()), None)
        .unwrap();

    // Another skill cannot register the same tunnel
    let result = router.register("uuid-1", "notion", Some("Notion".into()), None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("already owned"));

    // Same skill can re-register (update)
    router
        .register("uuid-1", "gmail", Some("Gmail Updated".into()), None)
        .unwrap();
}

#[test]
fn test_unregister_ownership() {
    let router = WebhookRouter::new(None);
    router.register("uuid-1", "gmail", None, None).unwrap();

    // Another skill cannot unregister
    let result = router.unregister("uuid-1", "notion");
    assert!(result.is_err());

    // Owner can unregister
    router.unregister("uuid-1", "gmail").unwrap();
    assert_eq!(router.route("uuid-1"), None);
}

#[test]
fn test_unregister_skill() {
    let router = WebhookRouter::new(None);
    router.register("uuid-1", "gmail", None, None).unwrap();
    router.register("uuid-2", "gmail", None, None).unwrap();
    router.register("uuid-3", "notion", None, None).unwrap();

    router.unregister_skill("gmail");

    assert_eq!(router.route("uuid-1"), None);
    assert_eq!(router.route("uuid-2"), None);
    assert_eq!(router.route("uuid-3"), Some("notion".to_string()));
}

#[test]
fn test_list_for_skill() {
    let router = WebhookRouter::new(None);
    router.register("uuid-1", "gmail", None, None).unwrap();
    router.register("uuid-2", "notion", None, None).unwrap();
    router.register("uuid-3", "gmail", None, None).unwrap();

    let gmail_tunnels = router.list_for_skill("gmail");
    assert_eq!(gmail_tunnels.len(), 2);
    assert!(gmail_tunnels.iter().all(|t| t.skill_id == "gmail"));

    let notion_tunnels = router.list_for_skill("notion");
    assert_eq!(notion_tunnels.len(), 1);

    let empty = router.list_for_skill("nonexistent");
    assert!(empty.is_empty());
}

#[test]
fn test_record_request_and_response() {
    let router = WebhookRouter::new(None);
    let request = WebhookRequest {
        correlation_id: "corr-1".to_string(),
        tunnel_id: "tunnel-id-1".to_string(),
        tunnel_uuid: "uuid-1".to_string(),
        tunnel_name: "Inbox".to_string(),
        method: "POST".to_string(),
        path: "/hooks/test".to_string(),
        headers: HashMap::from([(String::from("x-test"), json!("1"))]),
        query: HashMap::from([(String::from("hello"), String::from("world"))]),
        body: "aGVsbG8=".to_string(),
    };
    let response = WebhookResponseData {
        correlation_id: "corr-1".to_string(),
        status_code: 204,
        headers: HashMap::from([(String::from("x-ok"), String::from("yes"))]),
        body: String::new(),
    };

    router.record_request(&request, Some("gmail".to_string()));
    router.record_response(&request, &response, Some("gmail".to_string()), None);

    let logs = router.list_logs(Some(10));
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].correlation_id, "corr-1");
    assert_eq!(logs[0].status_code, Some(204));
    assert_eq!(logs[0].skill_id.as_deref(), Some("gmail"));
    assert_eq!(logs[0].stage, "completed");
}

#[test]
fn test_clear_logs() {
    let router = WebhookRouter::new(None);
    router.record_parse_error(
        "corr-2".to_string(),
        Some("uuid-2".to_string()),
        Some("POST".to_string()),
        Some("/broken".to_string()),
        json!({ "broken": true }),
        "bad payload".to_string(),
    );

    assert_eq!(router.list_logs(Some(10)).len(), 1);
    assert_eq!(router.clear_logs(), 1);
    assert!(router.list_logs(Some(10)).is_empty());
}

#[test]
fn register_echo_and_route_returns_none_for_echo_targets() {
    let router = WebhookRouter::new(None);
    router
        .register_echo("uuid-echo", Some("Test Echo".into()), None)
        .unwrap();
    // Echo targets are target_kind="echo", route() only returns "skill" targets
    assert_eq!(router.route("uuid-echo"), None);
}

#[test]
fn registration_returns_full_tunnel_info() {
    let router = WebhookRouter::new(None);
    router
        .register(
            "uuid-1",
            "gmail",
            Some("My Tunnel".into()),
            Some("bt-1".into()),
        )
        .unwrap();
    let reg = router.registration("uuid-1").unwrap();
    assert_eq!(reg.tunnel_uuid, "uuid-1");
    assert_eq!(reg.skill_id, "gmail");
    assert_eq!(reg.tunnel_name.as_deref(), Some("My Tunnel"));
    assert_eq!(reg.backend_tunnel_id.as_deref(), Some("bt-1"));
}

#[test]
fn registration_returns_none_for_missing_uuid() {
    let router = WebhookRouter::new(None);
    assert!(router.registration("no-such").is_none());
}

#[test]
fn list_all_returns_all_registrations() {
    let router = WebhookRouter::new(None);
    router.register("u1", "s1", None, None).unwrap();
    router.register("u2", "s2", None, None).unwrap();
    let all = router.list_all();
    assert_eq!(all.len(), 2);
}

#[test]
fn list_logs_respects_limit() {
    let router = WebhookRouter::new(None);
    for i in 0..5 {
        router.record_parse_error(
            format!("corr-{i}"),
            None,
            None,
            None,
            json!({}),
            "error".into(),
        );
    }
    let logs = router.list_logs(Some(3));
    assert_eq!(logs.len(), 3);
}

#[test]
fn list_logs_default_limit() {
    let router = WebhookRouter::new(None);
    for i in 0..5 {
        router.record_parse_error(
            format!("corr-{i}"),
            None,
            None,
            None,
            json!({}),
            "err".into(),
        );
    }
    let logs = router.list_logs(None);
    assert_eq!(logs.len(), 5); // less than default limit of 100
}

#[test]
fn record_response_without_prior_request_creates_new_entry() {
    let router = WebhookRouter::new(None);
    let request = WebhookRequest {
        correlation_id: "corr-new".into(),
        tunnel_id: "tid".into(),
        tunnel_uuid: "uuid-new".into(),
        tunnel_name: "Test".into(),
        method: "POST".into(),
        path: "/test".into(),
        headers: HashMap::new(),
        query: HashMap::new(),
        body: String::new(),
    };
    let response = WebhookResponseData {
        correlation_id: "corr-new".into(),
        status_code: 200,
        headers: HashMap::new(),
        body: "ok".into(),
    };
    // No prior record_request — should still create a log entry
    router.record_response(&request, &response, None, None);
    let logs = router.list_logs(Some(10));
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].stage, "completed");
}

#[test]
fn record_response_with_error_sets_error_stage() {
    let router = WebhookRouter::new(None);
    let request = WebhookRequest {
        correlation_id: "corr-err".into(),
        tunnel_id: "tid".into(),
        tunnel_uuid: "uuid-err".into(),
        tunnel_name: "Test".into(),
        method: "POST".into(),
        path: "/test".into(),
        headers: HashMap::new(),
        query: HashMap::new(),
        body: String::new(),
    };
    let response = WebhookResponseData {
        correlation_id: "corr-err".into(),
        status_code: 500,
        headers: HashMap::new(),
        body: String::new(),
    };
    router.record_request(&request, None);
    router.record_response(&request, &response, None, Some("handler crashed".into()));
    let logs = router.list_logs(Some(10));
    assert_eq!(logs[0].stage, "error");
    assert_eq!(logs[0].error_message.as_deref(), Some("handler crashed"));
}

#[test]
fn clear_logs_returns_zero_when_empty() {
    let router = WebhookRouter::new(None);
    assert_eq!(router.clear_logs(), 0);
}

#[test]
fn subscribe_debug_events_does_not_panic() {
    let router = WebhookRouter::new(None);
    let _rx = router.subscribe_debug_events();
}

#[test]
fn persist_and_load_roundtrip() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();

    let router = WebhookRouter::new(Some(path.clone()));
    router
        .register("uuid-p1", "skill-a", Some("Tunnel A".into()), None)
        .unwrap();
    router
        .register("uuid-p2", "skill-b", None, Some("bt-2".into()))
        .unwrap();

    // Load from disk
    let router2 = WebhookRouter::new(Some(path));
    assert_eq!(router2.list_all().len(), 2);
    assert!(router2.registration("uuid-p1").is_some());
    assert!(router2.registration("uuid-p2").is_some());
}

#[test]
fn unregister_nonexistent_tunnel_is_noop() {
    let router = WebhookRouter::new(None);
    // Should not error even though tunnel doesn't exist
    router.unregister("no-such", "any-skill").unwrap();
}

#[test]
fn unregister_skill_with_no_tunnels_is_noop() {
    let router = WebhookRouter::new(None);
    router.register("u1", "other", None, None).unwrap();
    router.unregister_skill("nonexistent");
    assert_eq!(router.list_all().len(), 1);
}

#[test]
fn record_parse_error_creates_entry_with_parse_error_stage() {
    let router = WebhookRouter::new(None);
    router.record_parse_error(
        "corr-p".into(),
        Some("uuid-p".into()),
        Some("GET".into()),
        Some("/bad".into()),
        json!({"raw": true}),
        "malformed body".into(),
    );
    let logs = router.list_logs(Some(1));
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].stage, "parse_error");
    assert_eq!(logs[0].status_code, Some(400));
    assert_eq!(logs[0].error_message.as_deref(), Some("malformed body"));
}

#[test]
fn truncate_logs_respects_max() {
    let router = WebhookRouter::new(None);
    for i in 0..(MAX_DEBUG_LOG_ENTRIES + 10) {
        router.record_parse_error(format!("c-{i}"), None, None, None, json!({}), "e".into());
    }
    let logs = router.list_logs(Some(MAX_DEBUG_LOG_ENTRIES + 100));
    assert!(logs.len() <= MAX_DEBUG_LOG_ENTRIES);
}

#[test]
fn register_agent_persists_agent_id_and_name() {
    let router = WebhookRouter::new(None);
    router
        .register_agent(
            "uuid-a1",
            Some("agent-42".into()),
            Some("My Agent".into()),
            None,
        )
        .unwrap();

    let reg = router.registration("uuid-a1").unwrap();
    assert_eq!(reg.target_kind, "agent");
    assert_eq!(reg.agent_id.as_deref(), Some("agent-42"));
    assert_eq!(reg.tunnel_name.as_deref(), Some("My Agent"));
}

#[test]
fn register_agent_same_id_succeeds() {
    let router = WebhookRouter::new(None);
    router
        .register_agent("uuid-a2", Some("agent-1".into()), None, None)
        .unwrap();
    // Re-register with the same agent_id should succeed.
    router
        .register_agent(
            "uuid-a2",
            Some("agent-1".into()),
            Some("Updated".into()),
            None,
        )
        .unwrap();

    let reg = router.registration("uuid-a2").unwrap();
    assert_eq!(reg.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(reg.tunnel_name.as_deref(), Some("Updated"));
}

#[test]
fn register_agent_rejects_different_agent_id() {
    let router = WebhookRouter::new(None);
    router
        .register_agent("uuid-a3", Some("agent-A".into()), None, None)
        .unwrap();

    let err = router
        .register_agent("uuid-a3", Some("agent-B".into()), None, None)
        .unwrap_err();
    assert!(err.contains("already bound"));

    // Original agent_id is preserved.
    let reg = router.registration("uuid-a3").unwrap();
    assert_eq!(reg.agent_id.as_deref(), Some("agent-A"));
}
