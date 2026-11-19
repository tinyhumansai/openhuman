use super::*;
use crate::security::policy::AutonomyLevel;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Mutex;
use tinyagents_harness::host::security_gate::{ContentOrigin, ScreenOutcome, SecurityGate};
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
        tool("spawn_subagent", PermissionLevel::Execute, false),
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

#[cfg(feature = "modules")]
#[derive(Debug, Default)]
struct ScriptedCommandGateway(Mutex<Vec<(String, crate::security::policy::CommandStage)>>);

#[cfg(feature = "modules")]
impl crate::security::policy::Sealed for ScriptedCommandGateway {}

#[cfg(feature = "modules")]
#[async_trait]
impl crate::security::policy::CommandGateway for ScriptedCommandGateway {
    async fn decide(
        &self,
        _policy: &SecurityPolicy,
        query: &crate::security::policy::CommandQuery<'_>,
    ) -> Result<crate::security::policy::CommandVerdict, crate::security::policy::Refusal> {
        self.0
            .lock()
            .unwrap()
            .push((query.command.to_owned(), query.stage));
        Ok(crate::security::policy::CommandVerdict {
            class: crate::security::policy::CommandClass::Read,
            gate: PolicyGateDecision::Allow,
        })
    }
}

#[cfg(feature = "modules")]
fn gate_with_command_gateway(
    autonomy: AutonomyLevel,
    gateway: Arc<ScriptedCommandGateway>,
) -> OpenHumanSecurityGate {
    let mut policy = SecurityPolicy {
        autonomy,
        ..SecurityPolicy::default()
    };
    policy.command_gateway = Some(gateway);
    OpenHumanSecurityGate::new(Arc::new(policy), registry())
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
async fn disabled_autonomy_allows_registered_actions_without_tinysecurity() {
    let policy = Arc::new(SecurityPolicy {
        enabled: false,
        ..SecurityPolicy::default()
    });
    let decision = OpenHumanSecurityGate::new(policy, registry())
        .authorize_tool(&req("write_file", json!({ "path": "notes.md" })))
        .await
        .expect("disabled autonomy should not require TinySecurity");

    assert_eq!(decision, GateDecision::Allow);
}

#[tokio::test]
async fn disabled_autonomy_still_denies_an_unregistered_tool() {
    let policy = Arc::new(SecurityPolicy {
        enabled: false,
        ..SecurityPolicy::default()
    });
    let decision = OpenHumanSecurityGate::new(policy, registry())
        .authorize_tool(&req("definitely_not_a_tool", json!({})))
        .await
        .unwrap();
    assert!(!decision.is_allowed());
}

async fn cron_decision(gate: OpenHumanSecurityGate, tool_name: &str) -> GateDecision {
    let core =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let origin = crate::core::runtime::turn_origin::AgentTurnOrigin::TrustedAutomation {
        job_id: "cron-security-test".into(),
        source: crate::core::runtime::turn_origin::TrustedAutomationSource::Cron,
    };
    crate::core::runtime::CoreContext::scope_with_turn_origin(
        core,
        Some(origin),
        gate.authorize_tool(&req(tool_name, json!({}))),
    )
    .await
    .expect("authorization")
}

#[tokio::test]
async fn cron_headless_settlement_allows_only_spawn_control() {
    let ordinary = cron_decision(gate(AutonomyLevel::Full), "spawn_subagent").await;
    assert!(!ordinary.is_allowed(), "ordinary Cron remains read-only");

    let opted_in = cron_decision(
        gate(AutonomyLevel::Full).with_headless_descendant_settlement(true),
        "spawn_subagent",
    )
    .await;
    assert_eq!(opted_in, GateDecision::Allow);

    let read_only = cron_decision(
        OpenHumanSecurityGate::new(policy(AutonomyLevel::ReadOnly), registry())
            .with_headless_descendant_settlement(true),
        "spawn_subagent",
    )
    .await;
    assert!(
        !read_only.is_allowed(),
        "headless settlement must not override the agent's autonomy tier"
    );

    let write = cron_decision(
        gate(AutonomyLevel::Full).with_headless_descendant_settlement(true),
        "write_file",
    )
    .await;
    assert!(!write.is_allowed(), "child action tools remain read-only");

    let shell = cron_decision(
        gate(AutonomyLevel::Full).with_headless_descendant_settlement(true),
        SHELL_TOOL,
    )
    .await;
    assert!(!shell.is_allowed(), "shell stays denied for opted-in Cron");
}

#[tokio::test]
async fn cron_headless_settlement_does_not_allow_effectful_spawn_aliases() {
    let effectful = Arc::new(vec![tool("spawn_subagent", PermissionLevel::Execute, true)]);
    let decision = cron_decision(
        OpenHumanSecurityGate::new(policy(AutonomyLevel::Full), vec![effectful])
            .with_headless_descendant_settlement(true),
        "spawn_subagent",
    )
    .await;
    assert!(
        !decision.is_allowed(),
        "external-effect classification wins"
    );
}

// ── scheduled-actions opt-in (`TrustedAutomationSource::CronActions`) ──

async fn scheduled_decision(
    gate: OpenHumanSecurityGate,
    source: crate::core::runtime::turn_origin::TrustedAutomationSource,
    depth: Option<usize>,
    tool_name: &str,
    args: serde_json::Value,
) -> GateDecision {
    let core =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let origin = crate::core::runtime::turn_origin::AgentTurnOrigin::TrustedAutomation {
        job_id: "cron-actions-test".into(),
        source,
    };
    let request = req(tool_name, args);
    let authorize = crate::core::runtime::CoreContext::scope_with_turn_origin(
        core,
        Some(origin),
        gate.authorize_tool(&request),
    );
    match depth {
        Some(depth) => {
            tinyagents_harness::context::with_run_depth(
                tinyagents_harness::context::RunDepth {
                    depth,
                    max_depth: 3,
                },
                authorize,
            )
            .await
        }
        None => authorize.await,
    }
    .expect("authorization")
}

fn disabled_autonomy_gate() -> OpenHumanSecurityGate {
    OpenHumanSecurityGate::new(
        Arc::new(SecurityPolicy {
            enabled: false,
            ..SecurityPolicy::default()
        }),
        registry(),
    )
}

#[tokio::test]
async fn opted_in_scheduled_actions_root_run_may_use_an_acting_tool() {
    use crate::core::runtime::turn_origin::TrustedAutomationSource;
    let decision = scheduled_decision(
        disabled_autonomy_gate().with_scheduled_actions(true),
        TrustedAutomationSource::CronActions,
        Some(0),
        "write_file",
        json!({ "path": "notes.md" }),
    )
    .await;
    assert_eq!(decision, GateDecision::Allow);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn opted_in_scheduled_actions_root_run_may_run_shell_through_tinysecurity() {
    use crate::core::runtime::turn_origin::TrustedAutomationSource;
    let gateway = Arc::new(ScriptedCommandGateway::default());
    let decision = scheduled_decision(
        gate_with_command_gateway(AutonomyLevel::Full, gateway.clone())
            .with_scheduled_actions(true),
        TrustedAutomationSource::CronActions,
        Some(0),
        SHELL_TOOL,
        json!({ "command": "cargo test" }),
    )
    .await;
    assert_eq!(decision, GateDecision::Allow);
    // The lift skips only the read-only cron stage: the command still went
    // through TinySecurity's analysis.
    assert_eq!(
        gateway.0.lock().unwrap().as_slice(),
        &[(
            "cargo test".to_owned(),
            crate::security::policy::CommandStage::Authorize
        )]
    );
}

#[cfg(feature = "web3")]
#[tokio::test]
async fn scheduled_actions_root_run_may_execute_a_prepared_wallet_transfer_but_plain_cron_may_not()
{
    use crate::core::runtime::turn_origin::TrustedAutomationSource;
    use crate::web3::wallet::automation_tools::{WalletBalancesTool, WalletExecutePreparedTool};
    let wallet_gate = || {
        OpenHumanSecurityGate::new(
            Arc::new(SecurityPolicy {
                enabled: false,
                ..SecurityPolicy::default()
            }),
            vec![Arc::new(vec![
                Box::new(WalletBalancesTool) as Box<dyn Tool>,
                Box::new(WalletExecutePreparedTool) as Box<dyn Tool>,
            ])],
        )
        .with_scheduled_actions(true)
    };
    let args = json!({ "quoteId": "q", "confirmed": true });
    for tool_name in ["wallet_balances", "wallet_execute_prepared"] {
        let allowed = scheduled_decision(
            wallet_gate(),
            TrustedAutomationSource::CronActions,
            Some(0),
            tool_name,
            args.clone(),
        )
        .await;
        assert_eq!(allowed, GateDecision::Allow, "{tool_name} for CronActions");
    }
    let denied = scheduled_decision(
        wallet_gate(),
        TrustedAutomationSource::Cron,
        Some(0),
        "wallet_execute_prepared",
        args.clone(),
    )
    .await;
    assert!(!denied.is_allowed(), "plain Cron must stay refused");
    let sub = scheduled_decision(
        wallet_gate(),
        TrustedAutomationSource::CronActions,
        Some(1),
        "wallet_execute_prepared",
        args,
    )
    .await;
    assert!(!sub.is_allowed(), "a sub-agent must stay read-only");
}

#[tokio::test]
async fn cron_job_without_the_scheduled_actions_opt_in_still_denies_shell_and_writes() {
    use crate::core::runtime::turn_origin::TrustedAutomationSource;
    for (tool_name, args) in [
        (SHELL_TOOL, json!({ "command": "echo hi" })),
        ("write_file", json!({ "path": "notes.md" })),
    ] {
        // A plain Cron origin is read-only even on a session marked for the
        // opt-in and at the root.
        let plain = scheduled_decision(
            disabled_autonomy_gate().with_scheduled_actions(true),
            TrustedAutomationSource::Cron,
            Some(0),
            tool_name,
            args.clone(),
        )
        .await;
        match plain {
            GateDecision::Deny { reason } => assert!(
                reason.contains("Scheduled agent turns may use read-only tools only"),
                "{reason}"
            ),
            other => panic!("{tool_name}: expected the cron denial, got {other:?}"),
        }
        // The origin alone does not lift it either: the session must be the
        // one the scheduler marked.
        let unmarked = scheduled_decision(
            disabled_autonomy_gate(),
            TrustedAutomationSource::CronActions,
            Some(0),
            tool_name,
            args.clone(),
        )
        .await;
        assert!(!unmarked.is_allowed(), "{tool_name} on an unmarked session");
    }
}

#[tokio::test]
async fn scheduled_actions_subagent_and_unscoped_runs_stay_read_only() {
    use crate::core::runtime::turn_origin::TrustedAutomationSource;
    for depth in [Some(1), None] {
        let decision = scheduled_decision(
            disabled_autonomy_gate().with_scheduled_actions(true),
            TrustedAutomationSource::CronActions,
            depth,
            "write_file",
            json!({ "path": "notes.md" }),
        )
        .await;
        assert!(
            !decision.is_allowed(),
            "depth {depth:?} must not inherit the lift"
        );
    }
}

// Fails closed only while security is enforced: a build without the
// security module has security disabled (`crate::security::switch`).
#[cfg(feature = "security-module")]
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

#[tokio::test]
#[cfg(feature = "modules")]
async fn shell_authorization_uses_command_without_tinybox_facts() {
    let gateway = Arc::new(ScriptedCommandGateway::default());
    let gate = gate_with_command_gateway(AutonomyLevel::Full, gateway.clone());
    let decision = gate
        .authorize_tool(&req(SHELL_TOOL, json!({"command": "ls -la"})))
        .await
        .expect("authorization result");
    assert!(decision.is_allowed());
    assert_eq!(
        gateway.0.lock().unwrap().as_slice(),
        &[(
            "ls -la".into(),
            crate::security::policy::CommandStage::Authorize
        )]
    );
}

#[tokio::test]
#[cfg(feature = "modules")]
async fn shell_authorization_ignores_stale_facts_and_uses_request_command() {
    let gateway = Arc::new(ScriptedCommandGateway::default());
    let gate = gate_with_command_gateway(AutonomyLevel::Full, gateway.clone());
    let args = json!({"command": "ls -la"});
    let context = json!({
        "tool_name": "shell",
        "request_arguments": {"command": "other command"},
        "effective_arguments": {"command": "cat ~/.ssh/id_rsa"},
        "tool_facts": {"analysis": "stale"},
    });
    let decision = gate
        .authorize_tool_with_context(&req(SHELL_TOOL, args), Some(&context))
        .await
        .expect("authorization result");
    assert!(decision.is_allowed());
    assert_eq!(gateway.0.lock().unwrap()[0].0, "ls -la");
}

#[path = "security_gate_screen_and_policy_tests.rs"]
mod screen_and_policy;

// -- security disabled (`crate::security::switch`) -----------------

/// What an enabled-autonomy gate decides for an acting tool and a shell command.
async fn acting_decisions() -> (GateDecision, GateDecision) {
    let policy = Arc::new(SecurityPolicy {
        enabled: true,
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    });
    let gate = OpenHumanSecurityGate::new(policy, registry());
    let write = gate
        .authorize_tool(&req("write_file", json!({ "path": "notes.md" })))
        .await
        .unwrap();
    let shell = gate
        .authorize_tool(&req(SHELL_TOOL, json!({ "command": "rm -rf build" })))
        .await
        .unwrap();
    (write, shell)
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn a_build_without_the_module_authorizes_tools_without_classifying_them() {
    let (write, shell) = acting_decisions().await;
    assert_eq!(write, GateDecision::Allow);
    assert_eq!(shell, GateDecision::Allow);
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn security_configured_off_authorizes_tools_without_classifying_them() {
    let _off = crate::security::switch::off_for_this_thread();
    let (write, shell) = acting_decisions().await;
    assert_eq!(write, GateDecision::Allow);
    assert_eq!(shell, GateDecision::Allow);
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn an_enforcing_gate_denies_a_tool_the_module_cannot_classify() {
    use crate::core::runtime::{CoreContext, DomainSet};
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::credentials::test_support::plaintext_fixture_config(temp.path());
    config.modules.enabled = false;
    let context = CoreContext::for_test_with_config(DomainSet::full(), config);
    let (write, _) = CoreContext::scope(context, acting_decisions()).await;
    assert!(!write.is_allowed());
    assert!(
        write
            .denial_reason()
            .expect("a Deny carries a reason")
            .contains(tinytools::status::POLICY_BLOCKED_MARKER),
        "{write:?}"
    );
}

/// Local-only privacy mode still refuses a shell command that may reach the
/// network when security is disabled; a local command still runs.
async fn local_only_shell_decisions() -> (GateDecision, GateDecision) {
    let _mode =
        crate::security::live_policy::test_privacy_scope(crate::config::PrivacyMode::LocalOnly);
    let gate = gate(AutonomyLevel::Full);
    let network = gate
        .authorize_tool(&req(
            SHELL_TOOL,
            json!({ "command": "curl https://example.com" }),
        ))
        .await
        .unwrap();
    let local = gate
        .authorize_tool(&req(SHELL_TOOL, json!({ "command": "ls -la" })))
        .await
        .unwrap();
    (network, local)
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn a_build_without_the_module_keeps_local_only_for_the_shell_tool() {
    let (network, local) = local_only_shell_decisions().await;
    assert!(!network.is_allowed());
    assert!(network
        .denial_reason()
        .is_some_and(|reason| reason.contains("Local-only privacy mode")));
    assert_eq!(local, GateDecision::Allow);
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn security_configured_off_keeps_local_only_for_the_shell_tool() {
    let _off = crate::security::switch::off_for_this_thread();
    let (network, local) = local_only_shell_decisions().await;
    assert!(!network.is_allowed());
    assert!(network
        .denial_reason()
        .is_some_and(|reason| reason.contains("Local-only privacy mode")));
    assert_eq!(local, GateDecision::Allow);
}
