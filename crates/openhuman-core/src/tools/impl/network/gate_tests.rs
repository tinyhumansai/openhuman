use super::*;
use crate::security::policy::native_commands_stub::StubGateway;
use crate::security::policy::Refusal;
use crate::security::{AutonomyLevel, SecurityPolicy};
use serde_json::json;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool};
use tinytools_std::network::{CurlTool, HttpRequestTool, PushoverTool, WebFetchTool};

/// A policy whose command gateway answers class questions with the tier table,
/// so these tests need not load the security module.
fn tiered(autonomy: AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        autonomy,
        ..SecurityPolicy::default()
    }
    .with_gateway(StubGateway::tiered())
}

fn policy(autonomy: AutonomyLevel, max_actions_per_hour: u32) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        max_actions_per_hour,
        ..tiered(autonomy)
    })
}

fn http(security: Arc<SecurityPolicy>) -> HttpRequestTool {
    super::super::host::http_request_tool(
        &crate::core::runtime::DispatchCtx::ambient(),
        security,
        vec!["example.com".into()],
        1_000_000,
        30,
    )
}

// --- the gate answers with the policy's own decisions -----------------------

#[test]
fn read_only_autonomy_cannot_act() {
    let gate: &dyn NetGate = &tiered(AutonomyLevel::ReadOnly);
    assert!(!gate.can_act());
}

#[test]
fn the_action_budget_is_shared_with_the_policy() {
    let security = policy(AutonomyLevel::Supervised, 1);
    let gate: &dyn NetGate = security.as_ref();
    assert!(!gate.is_rate_limited());
    assert!(gate.record_action());
    assert!(gate.is_rate_limited());
    assert!(!gate.record_action());
}

#[test]
fn supervised_network_asks_for_approval_and_read_only_does_not() {
    let supervised = policy(AutonomyLevel::Supervised, 100);
    let read_only = policy(AutonomyLevel::ReadOnly, 100);
    assert_eq!(
        NetGate::network_needs_approval(supervised.as_ref()),
        supervised.gate_decision(CommandClass::Network) == GateDecision::Prompt
    );
    assert!(NetGate::network_needs_approval(supervised.as_ref()));
    assert!(!NetGate::network_needs_approval(read_only.as_ref()));
}

#[test]
fn the_network_gate_asks_the_module_for_the_network_class() {
    let gateway = StubGateway::prompt();
    let security = SecurityPolicy::default().with_gateway(gateway.clone());
    assert!(NetGate::network_needs_approval(&security));
    let asked = gateway.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].declared, Some(CommandClass::Network));
    assert!(
        asked[0].command.is_empty(),
        "a class question has no command"
    );
}

#[test]
fn a_module_that_cannot_answer_refuses_the_call_and_asks_for_approval() {
    // Fail closed: an unanswered tier question must neither run the request nor
    // read as "no approval needed".
    let security =
        SecurityPolicy::default().with_gateway(StubGateway::refuse(Refusal::Unavailable));
    assert!(!NetGate::can_act(&security));
    assert!(NetGate::network_needs_approval(&security));
}

#[test]
fn a_blocked_tier_stops_the_call_before_the_network() {
    let blocked = SecurityPolicy::default().with_gateway(StubGateway::gate(
        CommandClass::Network,
        GateDecision::Block,
    ));
    assert!(!NetGate::can_act(&blocked));
    assert!(!NetGate::network_needs_approval(&blocked));
}

// --- privacy mode ------------------------------------------------------------

#[test]
fn local_only_privacy_mode_refuses_with_the_policy_blocked_marker() {
    let _mode = super::super::local_only_scope();
    let msg = NetGate::local_only_block(&tiered(AutonomyLevel::Supervised), "example.com")
        .expect("LocalOnly blocks a network fetch");
    assert!(msg.contains("[policy-blocked]"), "got: {msg}");
    assert!(msg.contains("Local-only"), "got: {msg}");
    assert!(msg.contains("example.com"), "names the host: {msg}");
}

#[test]
fn standard_privacy_mode_lets_the_request_through() {
    let _mode =
        crate::security::live_policy::test_privacy_scope(crate::config::PrivacyMode::Standard);
    assert!(NetGate::local_only_block(&SecurityPolicy::default(), "example.com").is_none());
}

// --- the tools, through the real policy ---------------------------------------

#[tokio::test]
async fn http_request_blocks_readonly_mode() {
    let tool = http(policy(AutonomyLevel::ReadOnly, 100));
    let result = tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"));
}

#[tokio::test]
async fn http_request_blocks_when_rate_limited() {
    let tool = http(policy(AutonomyLevel::Supervised, 0));
    let result = tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("rate limit"));
}

#[tokio::test]
async fn http_request_is_blocked_under_local_only_privacy_mode() {
    // Privacy epic S7 (#4441): under LocalOnly the request is refused with a
    // `[policy-blocked]` result before URL validation / network.
    let _mode = super::super::local_only_scope();
    let result = http(Arc::new(tiered(AutonomyLevel::Supervised)))
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("[policy-blocked]"),
        "got: {}",
        result.output()
    );
    assert!(
        result.output().contains("Local-only"),
        "got: {}",
        result.output()
    );
}

#[tokio::test]
async fn web_fetch_and_curl_are_blocked_under_local_only_privacy_mode() {
    let _mode = super::super::local_only_scope();
    let security = Arc::new(tiered(AutonomyLevel::Supervised));
    let fetch = super::super::host::web_fetch_tool(
        &crate::core::runtime::DispatchCtx::ambient(),
        security.clone(),
        vec!["example.com".into()],
        None,
        None,
    );
    let curl = CurlTool::new(
        security,
        vec!["example.com".into()],
        std::env::temp_dir(),
        "downloads".into(),
        1024,
        30,
    );
    for result in [
        fetch
            .execute(json!({"url": "https://example.com/data"}))
            .await
            .unwrap(),
        curl.execute(json!({"url": "https://example.com/x"}))
            .await
            .unwrap(),
    ] {
        assert!(result.is_error);
        assert!(
            result.output().contains("[policy-blocked]"),
            "got: {}",
            result.output()
        );
        assert!(
            result.output().contains("Local-only"),
            "got: {}",
            result.output()
        );
    }
}

#[tokio::test]
async fn pushover_honours_read_only_and_the_rate_limit() {
    let read_only = PushoverTool::new(policy(AutonomyLevel::ReadOnly, 100), "/tmp".into());
    let result = read_only
        .execute(json!({"message": "hello"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"));

    let limited = PushoverTool::new(policy(AutonomyLevel::Full, 0), "/tmp".into());
    let result = limited.execute(json!({"message": "hello"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("rate limit"));
}

#[test]
fn supervised_http_request_is_external_effect_for_approval_gate() {
    let tool = http(policy(AutonomyLevel::Supervised, 100));
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert!(tool.external_effect_with_args(&json!({
        "url": "https://example.com/api",
        "method": "POST",
        "headers": { "Authorization": "Bearer token" },
        "body": "{}"
    })));
}

#[test]
fn readonly_http_request_is_not_external_effect_because_execute_blocks() {
    let tool = http(policy(AutonomyLevel::ReadOnly, 100));
    assert!(!tool.external_effect_with_args(&json!({
        "url": "https://example.com/api",
        "method": "GET"
    })));
}

#[test]
fn web_fetch_is_read_only_and_never_prompts() {
    let tool: WebFetchTool = super::super::host::web_fetch_tool(
        &crate::core::runtime::DispatchCtx::ambient(),
        policy(AutonomyLevel::Supervised, 100),
        vec![],
        None,
        None,
    );
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
}

// --- egress descriptor -------------------------------------------------------

#[test]
fn egress_descriptor_reports_host_and_url_kind_when_bare() {
    // A bodyless, headerless GET discloses only the destination host + URL.
    let desc = network_egress_descriptor("api.example.com", false, false);
    assert_eq!(desc.provider_slug, "network");
    assert_eq!(desc.service, "api.example.com");
    assert!(desc.is_external);
    assert_eq!(desc.data_kinds, vec![DataKind::Url]);
}

#[test]
fn egress_descriptor_adds_tool_arguments_for_body() {
    let desc = network_egress_descriptor("api.example.com", true, false);
    assert!(desc.data_kinds.contains(&DataKind::ToolArguments));
    assert!(!desc.data_kinds.contains(&DataKind::Metadata));
}

#[test]
fn egress_descriptor_reports_headers_as_metadata_even_without_body() {
    // Regression for the under-reporting fix: a header-only call (e.g. an
    // Authorization token, no body) must still disclose the header metadata.
    let desc = network_egress_descriptor("api.example.com", false, true);
    assert!(desc.data_kinds.contains(&DataKind::Metadata));
}

#[test]
fn egress_descriptor_reports_both_body_and_headers() {
    let desc = network_egress_descriptor("api.example.com", true, true);
    assert!(desc.data_kinds.contains(&DataKind::ToolArguments));
    assert!(desc.data_kinds.contains(&DataKind::Metadata));
}
