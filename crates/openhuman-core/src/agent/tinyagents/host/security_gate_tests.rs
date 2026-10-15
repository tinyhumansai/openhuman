use super::*;
use crate::security::policy::native_commands_stub::{Asked, StubGateway};
use crate::security::policy::{AutonomyLevel, CommandFlow, CommandStage, Refusal};
use async_trait::async_trait;
use serde_json::json;
use tinyagents_harness::host::security_gate::{ContentOrigin, ScreenOutcome, SecurityGate};
use tinybox_bus::CommandClass;
use tinytools::ToolResult;

/// Minimal registered tool. `execute` is unreachable: this adapter answers
/// questions about tools, it never runs them.
struct FakeTool {
    name: &'static str,
    permission: PermissionLevel,
    external: bool,
}

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "test tool"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }
    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }
    fn external_effect(&self) -> bool {
        self.external
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        unreachable!("the security gate never executes a tool")
    }
}

fn tool(name: &'static str, permission: PermissionLevel, external: bool) -> Box<dyn Tool> {
    Box::new(FakeTool {
        name,
        permission,
        external,
    })
}

fn registry() -> Vec<Arc<Vec<Box<dyn Tool>>>> {
    vec![Arc::new(vec![
        tool("read_file", PermissionLevel::ReadOnly, false),
        tool("write_file", PermissionLevel::Write, false),
        tool(SHELL_TOOL, PermissionLevel::Execute, false),
    ])]
}

fn policy(autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy,
        ..SecurityPolicy::default()
    })
}

fn gate(autonomy: AutonomyLevel) -> OpenHumanSecurityGate {
    OpenHumanSecurityGate::new(policy(autonomy), registry())
}

fn req(tool_name: &str, args: serde_json::Value) -> ToolCallRequest {
    ToolCallRequest::new(tool_name, args, "lead")
}

#[tokio::test]
async fn there_is_no_audit_id_to_drain_when_nothing_was_prompted() {
    // No approval gate is installed in tests, so nothing parks and nothing
    // is stashed. The point of the assertion is that `take_audit_request_id`
    // is a drain, not a source: a caller can never mistake "not prompted"
    // for "there is a row to close", which is what would push it into
    // issuing a second `intercept_audited` and raising a duplicate card.
    let gate = gate(AutonomyLevel::Full);
    let _ = gate
        .authorize_tool(&req("read_file", json!({ "path": "notes.md" })))
        .await
        .expect("authorize");

    assert_eq!(gate.take_audit_request_id("any-call-id"), None);
}

#[test]
fn draining_an_audit_id_yields_it_exactly_once() {
    // An id is valid for exactly one `record_execution`; handing the same
    // one out twice would write two terminal rows for a single approval.
    let gate = gate(AutonomyLevel::Full);
    gate.pending_audit
        .lock()
        .expect("uncontended")
        .insert("call-1".to_string(), "request-9".to_string());

    assert_eq!(
        gate.take_audit_request_id("call-1"),
        Some("request-9".to_string())
    );
    assert_eq!(gate.take_audit_request_id("call-1"), None);
}

#[tokio::test]
async fn a_read_only_tool_is_allowed_in_every_tier() {
    for tier in [
        AutonomyLevel::ReadOnly,
        AutonomyLevel::Supervised,
        AutonomyLevel::Full,
    ] {
        let decision = gate(tier)
            .authorize_tool(&req("read_file", json!({ "path": "notes.md" })))
            .await
            .unwrap();
        assert_eq!(decision, GateDecision::Allow, "tier {tier:?}");
    }
}

#[tokio::test]
async fn read_only_autonomy_denies_an_acting_tool() {
    let decision = gate(AutonomyLevel::ReadOnly)
        .authorize_tool(&req("write_file", json!({ "path": "a.txt" })))
        .await
        .unwrap();
    assert!(!decision.is_allowed());
    let reason = decision.denial_reason().expect("a Deny carries a reason");
    assert!(
        reason.contains(tinytools::status::POLICY_BLOCKED_MARKER),
        "the machine-recognisable marker must survive rendering: {reason}"
    );
}

#[tokio::test]
async fn an_unregistered_tool_is_denied_not_allowed() {
    // Fail-closed: without the registered Tool there is no permission level
    // to reason about, so "unknown" must never mean "fine".
    let decision = gate(AutonomyLevel::Full)
        .authorize_tool(&req("definitely_not_a_tool", json!({})))
        .await
        .unwrap();
    assert!(!decision.is_allowed());
    assert!(decision
        .denial_reason()
        .expect("denial reason")
        .contains("definitely_not_a_tool"));
}

#[tokio::test]
async fn an_empty_registry_denies_everything() {
    let gate = OpenHumanSecurityGate::new(policy(AutonomyLevel::Full), Vec::new());
    assert!(!gate
        .authorize_tool(&req("read_file", json!({})))
        .await
        .unwrap()
        .is_allowed());
}

fn shell_gate(
    autonomy: AutonomyLevel,
    gateway: Arc<StubGateway>,
) -> (OpenHumanSecurityGate, Arc<StubGateway>) {
    let policy = SecurityPolicy {
        autonomy,
        ..SecurityPolicy::default()
    }
    .with_gateway(gateway.clone());
    (
        OpenHumanSecurityGate::new(Arc::new(policy), registry()),
        gateway,
    )
}

#[tokio::test]
async fn a_shell_command_the_module_allows_runs_without_a_prompt() {
    // `shell` declares the coarse `Execute` permission level, so a tier check on
    // the tool alone would refuse `ls` in read-only mode. The module's answer
    // for the command itself is what decides, so a read still runs.
    for tier in [AutonomyLevel::ReadOnly, AutonomyLevel::Supervised] {
        let (gate, gateway) = shell_gate(tier, StubGateway::allow());
        let decision = gate
            .authorize_tool(&req(SHELL_TOOL, json!({"command": "ls -la"})))
            .await
            .unwrap();
        assert_eq!(decision, GateDecision::Allow, "tier {tier:?}");
        assert_eq!(gateway.asked().len(), 1);
    }
}

#[tokio::test]
async fn a_shell_command_the_module_refuses_is_denied_with_the_policy_marker() {
    for reason in [
        Refusal::ReadOnly,
        Refusal::Hidden,
        Refusal::Protected,
        Refusal::HighRisk,
        Refusal::RateLimit,
        Refusal::Unavailable,
    ] {
        let (gate, _) = shell_gate(AutonomyLevel::Full, StubGateway::refuse(reason));
        let decision = gate
            .authorize_tool(&req(SHELL_TOOL, json!({"command": "ls"})))
            .await
            .expect("authorization result");
        assert!(!decision.is_allowed(), "{reason:?} must deny");
    }
    let (gate, _) = shell_gate(AutonomyLevel::Full, StubGateway::refuse(Refusal::Hidden));
    let decision = gate
        .authorize_tool(&req(SHELL_TOOL, json!({"command": "echo $(whoami)"})))
        .await
        .unwrap();
    assert!(decision
        .denial_reason()
        .expect("denial reason")
        .contains(tinytools::status::POLICY_BLOCKED_MARKER));
}

#[tokio::test]
async fn the_gate_asks_before_approval_and_does_not_spend_the_run_budget() {
    let (gate, gateway) = shell_gate(AutonomyLevel::Full, StubGateway::allow());
    gate.authorize_tool(&req(SHELL_TOOL, json!({"command": "ls"})))
        .await
        .unwrap();
    assert_eq!(
        gateway.asked(),
        vec![Asked {
            command: "ls".into(),
            declared: None,
            flow: CommandFlow::Harness,
            stage: CommandStage::Authorize,
        }]
    );
}

#[tokio::test]
async fn shell_authorization_denies_a_call_without_a_command_and_never_asks() {
    let (gate, gateway) = shell_gate(AutonomyLevel::Full, StubGateway::allow());
    let decision = gate
        .authorize_tool(&req(SHELL_TOOL, json!({})))
        .await
        .unwrap();
    assert!(!decision.is_allowed());
    assert!(gateway.asked().is_empty());
}

#[tokio::test]
async fn the_declared_category_reaches_the_module_as_an_escalate_only_hint() {
    let policy = SecurityPolicy::default().with_gateway(StubGateway::prompt());
    let cases = [
        (
            json!({"command": "ls", "category": "destructive"}),
            Some(CommandClass::Destructive),
        ),
        (
            json!({"command": "ls", "category": "READ"}),
            Some(CommandClass::Read),
        ),
        (json!({"command": "ls", "category": "bogus"}), None),
        (json!({"command": "ls"}), None),
    ];
    for (args, expected) in cases {
        let gateway = StubGateway::prompt();
        let policy = SecurityPolicy {
            command_gateway: Some(gateway.clone()),
            ..policy.clone()
        };
        assert_eq!(
            OpenHumanSecurityGate::shell_decision(&policy, &args).await,
            Ok(PolicyGateDecision::Prompt)
        );
        assert_eq!(gateway.asked()[0].declared, expected, "{args}");
    }
}

#[tokio::test]
async fn a_module_prompt_is_reported_as_a_prompt_and_an_allow_as_an_allow() {
    for (stub, expected) in [
        (StubGateway::prompt(), PolicyGateDecision::Prompt),
        (StubGateway::allow(), PolicyGateDecision::Allow),
    ] {
        let policy = SecurityPolicy::default().with_gateway(stub);
        assert_eq!(
            OpenHumanSecurityGate::shell_decision(&policy, &json!({"command": "touch f"})).await,
            Ok(expected)
        );
    }
}

#[path = "security_gate_screen_and_policy_tests.rs"]
mod screen_and_policy;
