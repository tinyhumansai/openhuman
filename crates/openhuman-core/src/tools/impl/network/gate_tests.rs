use super::*;
use crate::security::policy::native_commands_stub::StubGateway;
use crate::security::policy::Refusal;
use crate::security::{AutonomyLevel, SecurityPolicy};
use serde_json::json;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool};
#[cfg(feature = "security-module")]
use tinytools_std::network::{CurlTool, PushoverTool};
use tinytools_std::network::{HttpRequestTool, WebFetchTool};

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

#[cfg(feature = "security-module")]
use crate::modules::security::gap::stub::StubPort;
#[cfg(feature = "security-module")]
use crate::modules::security::gap::{install_port, PortGuard};
#[cfg(feature = "security-module")]
use tinysecurity_bus::{BudgetDecision2, DenialReason, Verdict};

#[cfg(feature = "security-module")]
/// A module that refuses every transfer it is asked about (a restrictive privacy
/// mode); the module owns the truth table, this scripts one answer.
fn refusing_privacy() -> PortGuard {
    install_port(Arc::new(StubPort::allowing().privacy(|_| Verdict::Deny {
        reason: DenialReason::Privacy,
    })))
}

#[cfg(feature = "security-module")]
/// A module whose autonomy tier is read-only.
fn read_only_module() -> PortGuard {
    install_port(Arc::new(
        StubPort::allowing().budget(|_, _| BudgetDecision2::ReadOnlyMode),
    ))
}

// --- the gate answers with the policy's own decisions -----------------------

#[cfg(feature = "security-module")]
#[test]
fn read_only_autonomy_cannot_act() {
    let _module = read_only_module();
    let gate: &dyn NetGate = &tiered(AutonomyLevel::ReadOnly);
    assert!(!gate.can_act());
}

#[cfg(feature = "security-module")]
#[test]
fn the_gate_does_not_count_an_action_the_tool_gate_already_reserved() {
    // The hourly budget is spent once per call by the tool gate before the tool
    // runs; the in-tool questions neither ask the module nor refuse.
    let stub = Arc::new(
        StubPort::allowing().budget(|_, _| BudgetDecision2::RateLimited { max_per_hour: 1 }),
    );
    let _module = install_port(stub.clone());
    let security = policy(AutonomyLevel::Supervised, 1);
    let gate: &dyn NetGate = security.as_ref();
    assert!(!gate.is_rate_limited());
    assert!(gate.record_action());
    assert!(gate.record_action());
    assert!(stub.asked().is_empty());
}

#[test]
fn supervised_network_asks_for_approval_and_read_only_does_not() {
    let supervised = policy(AutonomyLevel::Supervised, 100);
    let read_only = policy(AutonomyLevel::ReadOnly, 100);
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

#[cfg(feature = "security-module")]
#[test]
fn local_only_privacy_mode_refuses_with_the_policy_blocked_marker() {
    let _module = refusing_privacy();
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

#[cfg(feature = "security-module")]
#[tokio::test]
async fn http_request_blocks_readonly_mode() {
    let _module = read_only_module();
    let tool = http(policy(AutonomyLevel::ReadOnly, 100));
    let result = tool
        .execute(json!({"url": "https://example.com"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"));
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn http_request_is_blocked_under_local_only_privacy_mode() {
    // Privacy epic S7 (#4441): under LocalOnly the request is refused with a
    // `[policy-blocked]` result before URL validation / network.
    let _module = refusing_privacy();
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

#[cfg(feature = "security-module")]
#[tokio::test]
async fn web_fetch_and_curl_are_blocked_under_local_only_privacy_mode() {
    let _module = refusing_privacy();
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

#[cfg(feature = "security-module")]
#[tokio::test]
async fn pushover_honours_the_read_only_tier() {
    let _module = read_only_module();
    let read_only = PushoverTool::new(policy(AutonomyLevel::ReadOnly, 100), "/tmp".into());
    let result = read_only
        .execute(json!({"message": "hello"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"));
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

/// The network gate on the released module, with no gateway stub, from a
/// runtime worker and from a plain thread: read-only cannot act, supervised
/// can and asks for approval.
#[cfg(feature = "security-module")]
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_network_gate_is_answered_by_the_released_module() {
    use crate::modules::security::released_support::Released;

    let released = Released::new();
    let read_only = Arc::new(released.policy(AutonomyLevel::ReadOnly, 100));
    let supervised = Arc::new(released.policy(AutonomyLevel::Supervised, 100));

    assert!(!NetGate::can_act(read_only.as_ref()));
    assert!(!NetGate::network_needs_approval(read_only.as_ref()));
    assert!(NetGate::can_act(supervised.as_ref()));
    assert!(NetGate::network_needs_approval(supervised.as_ref()));

    let (can_act, needs_approval) = {
        let supervised = supervised.clone();
        std::thread::spawn(move || {
            (
                NetGate::can_act(supervised.as_ref()),
                NetGate::network_needs_approval(supervised.as_ref()),
            )
        })
        .join()
        .unwrap()
    };
    assert!(can_act && needs_approval);
}

// --- the call's payload decides a Sensitive transfer ------------------------

fn routed_with(payload: Option<Assessment>) -> RoutedGate {
    RoutedGate {
        policy: policy(AutonomyLevel::Supervised, 100),
        chat: None,
        payload,
    }
}

#[cfg(feature = "security-module")]
#[test]
fn sensitive_mode_permits_a_fetch_assessed_clean_and_refuses_the_rest() {
    use crate::config::PrivacyMode;
    use crate::security::live_policy::test_privacy_scope;
    // The module decides from the assessment the gate forwards: clean passes,
    // anything else (identifying, or not assessed) is refused.
    let _module = install_port(Arc::new(StubPort::allowing().privacy(
        |request| match &request.risk {
            Some(risk)
                if risk.level == tinysecurity_bus::PrivacyRiskLevel2::None
                    && risk.categories.is_empty() =>
            {
                Verdict::Allow
            }
            _ => Verdict::Deny {
                reason: DenialReason::Privacy,
            },
        },
    )));
    let _mode = test_privacy_scope(PrivacyMode::Sensitive);
    let clean = routed_with(Some(Assessment {
        level: IdentificationRisk::None,
        categories: Vec::new(),
    }));
    assert!(clean.local_only_block("example.com").is_none());

    let identifying = routed_with(Some(Assessment {
        level: IdentificationRisk::High,
        categories: vec!["credential_or_identifier".into()],
    }));
    let refusal = identifying
        .local_only_block("example.com")
        .expect("a payload the redactor would change is refused");
    assert!(refusal.contains("Sensitive privacy mode"), "{refusal}");

    // Not assessed (module could not answer): never assumed clean.
    assert!(routed_with(None).local_only_block("example.com").is_some());
}

#[test]
fn other_modes_never_assess_the_arguments() {
    use crate::config::PrivacyMode;
    use crate::security::live_policy::test_privacy_scope;
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    let gate = RoutedGate::new(
        policy(AutonomyLevel::Supervised, 100),
        None,
        &json!({"url": "https://example.com/?k=Bearer abcdefghijklmnop"}),
    );
    assert!(gate.payload.is_none());
    assert!(gate.local_only_block("example.com").is_none());
}

/// The released module decides a real network call: a plain GET is permitted
/// under Sensitive, one whose headers carry a credential is refused.
#[cfg(feature = "security-module")]
#[test]
#[cfg(feature = "native-security-tests")]
fn sensitive_network_gate_on_the_released_archive() {
    crate::security::redact::tests::shared_runtime().block_on(async {
        use crate::config::PrivacyMode;
        use crate::security::live_policy::test_privacy_scope;
        let (_dir, config) = crate::security::redact::tests::released_config();
        crate::security::redact::tests::arm(&config).await;
        let _mode = test_privacy_scope(PrivacyMode::Sensitive);
        let security = policy(AutonomyLevel::Supervised, 100);

        let plain = RoutedGate::new(
            Arc::clone(&security),
            None,
            &json!({"url": "https://example.com/docs", "method": "GET"}),
        );
        assert!(plain.local_only_block("example.com").is_none());

        let secret = format!("Bearer {}", "abcdefghijklmnopqrstuvwxyz0123456789");
        let leaking = RoutedGate::new(
            security,
            None,
            &json!({"url": "https://example.com/", "headers": {"Authorization": secret}}),
        );
        assert!(leaking.local_only_block("example.com").is_some());
    });
}

fn destination_of<'a>(url: &'a str) -> Destination<'a> {
    Destination {
        url,
        host: "example.com",
        port: 443,
        method: "GET",
        headers: &[],
        body: None,
    }
}

/// With the module built in, every destination is the module's to judge: a
/// destination it refuses (the metadata address, which the address floor denies
/// whether or not a module is serving) comes back as the tool-facing policy
/// message, never as addresses for the tool to connect to.
#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_destination_the_module_refuses_is_a_policy_message_not_addresses() {
    let dir = tempfile::tempdir().unwrap();
    let security = Arc::new(SecurityPolicy {
        workspace_dir: dir.path().to_path_buf(),
        ..SecurityPolicy::default()
    });
    let destination = Destination {
        url: "http://169.254.169.254/latest/meta-data",
        host: "169.254.169.254",
        port: 80,
        method: "GET",
        headers: &[],
        body: None,
    };
    for answer in [
        NetGate::authorize_destination(&*security, &destination).await,
        RoutedGate::new(security.clone(), None, &json!({}))
            .authorize_destination(&destination)
            .await,
    ] {
        let refusal = answer
            .expect("the module owns the decision")
            .expect_err("a refused destination yields no addresses");
        assert!(
            refusal.starts_with(tinytools::status::POLICY_BLOCKED_MARKER),
            "{refusal}"
        );
    }
}

/// A build without the module denies network destinations; it cannot bypass the
/// module's destination policy by falling back to a tool-local DNS check.
#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn without_the_module_network_destination_is_denied() {
    let security = SecurityPolicy::default();
    let destination = destination_of("https://example.com/");
    let routed = RoutedGate::new(Arc::new(security.clone()), None, &json!({}));
    for answer in [
        NetGate::authorize_destination(&security, &destination).await,
        routed.authorize_destination(&destination).await,
    ] {
        let refusal = answer
            .expect("missing module must not fall back to tool-local DNS")
            .expect_err("missing module must deny");
        assert!(
            refusal.starts_with(tinytools::status::POLICY_BLOCKED_MARKER),
            "missing module refusal must carry the policy marker: {refusal}"
        );
    }
}
