//! Host-tool admission on the native module, configured the way production
//! configures it (approvals on, audit forwarded but not required): the module
//! answers the questions the hosted-root security gate used to answer itself.
//!
//! These pin the contract the gate relies on: the read-only tier, the hourly
//! budget (spent by acting tools and by shell runs through one ledger), the
//! scheduled-turn restriction and the shell class gate. They load the module
//! like the other native tests, so they run only where one is selected.

use super::super::native_tests::{native_approval_fixture, NativeFixture};
use super::*;
use crate::security::policy::{CommandFlow, CommandStage};
use crate::security::{AutonomyLevel, SecurityPolicy};
use tinysecurity_bus::{
    AccessTier, CallerContext, Check, DenialReason, EvaluateRequest, Origin, ToolCall,
    ToolPermission, Verdict,
};

fn policy(autonomy: AutonomyLevel, enabled: bool, budget: u32) -> SecurityPolicy {
    SecurityPolicy {
        enabled,
        autonomy,
        max_actions_per_hour: budget,
        ..SecurityPolicy::default()
    }
}

struct Gate {
    fixture: NativeFixture,
    state: ClientState,
}

impl Gate {
    async fn start() -> Self {
        Self {
            fixture: native_approval_fixture().await,
            state: ClientState::default(),
        }
    }

    /// The scope the host registers for `policy`, as the gate does.
    async fn scope(&mut self, policy: &SecurityPolicy) -> CommandPolicyId {
        let command = policy
            .native_command_policy(CommandFlow::Harness, CommandStage::Execute)
            .unwrap();
        register_cached_command_scope(&mut self.state, &self.fixture.proxy, command)
            .await
            .expect("scope registered")
    }

    async fn evaluate(&self, origin: Origin, tool: &str, checks: Vec<Check>) -> Verdict {
        let decision: tinysecurity_bus::Decision = self
            .fixture
            .proxy
            .call_confidential(
                names::methods::EVALUATE,
                (EvaluateRequest {
                    caller: CallerContext {
                        agent: "agent".into(),
                        thread: None,
                        flow: None,
                        call_id: "call".into(),
                        origin,
                        tier: AccessTier::Full,
                    },
                    call: ToolCall {
                        tool: tool.into(),
                        arguments_json: "{}".into(),
                        checks,
                    },
                },),
            )
            .await
            .expect("module answered");
        decision.verdict
    }
}

fn acting(spend: bool, scope: &CommandPolicyId) -> Check {
    Check::HostAction {
        permission: ToolPermission::Write,
        external_effect: false,
        spend,
        command_policy: Some(scope.clone()),
    }
}

fn reading(scope: &CommandPolicyId) -> Check {
    Check::HostAction {
        permission: ToolPermission::ReadOnly,
        external_effect: false,
        spend: false,
        command_policy: Some(scope.clone()),
    }
}

fn shell(command: &str, scope: &CommandPolicyId) -> Check {
    Check::HostShell {
        command: command.into(),
        declared_class: None,
        spend: false,
        command_policy: Some(scope.clone()),
    }
}

fn deny(reason: DenialReason) -> Verdict {
    Verdict::Deny { reason }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_read_only_tier_refuses_an_acting_tool_but_not_a_read() {
    let mut gate = Gate::start().await;
    let scope = gate
        .scope(&policy(AutonomyLevel::ReadOnly, true, 100))
        .await;
    assert_eq!(
        gate.evaluate(Origin::Operator, "write_file", vec![acting(true, &scope)])
            .await,
        deny(DenialReason::ReadOnlyMode)
    );
    assert_eq!(
        gate.evaluate(Origin::Operator, "read_file", vec![reading(&scope)])
            .await,
        Verdict::Allow
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn an_exhausted_hourly_budget_denies_acting_tools_but_not_reads() {
    let mut gate = Gate::start().await;
    let scope = gate.scope(&policy(AutonomyLevel::Full, true, 2)).await;
    for _ in 0..2 {
        assert_eq!(
            gate.evaluate(Origin::Operator, "write_file", vec![acting(true, &scope)])
                .await,
            Verdict::Allow
        );
    }
    assert_eq!(
        gate.evaluate(Origin::Operator, "write_file", vec![acting(true, &scope)])
            .await,
        deny(DenialReason::RateLimited { max_per_hour: 2 })
    );
    assert_eq!(
        gate.evaluate(Origin::Operator, "read_file", vec![reading(&scope)])
            .await,
        Verdict::Allow
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_budget_is_inert_while_the_autonomy_policy_is_disabled() {
    let mut gate = Gate::start().await;
    let scope = gate.scope(&policy(AutonomyLevel::ReadOnly, false, 1)).await;
    for _ in 0..3 {
        assert_eq!(
            gate.evaluate(Origin::Operator, "write_file", vec![acting(true, &scope)])
                .await,
            Verdict::Allow
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_scheduled_turn_may_use_read_only_tools_only() {
    let mut gate = Gate::start().await;
    let scope = gate.scope(&policy(AutonomyLevel::Full, true, 100)).await;
    assert_eq!(
        gate.evaluate(Origin::Cron, "read_file", vec![reading(&scope)])
            .await,
        Verdict::Allow
    );
    for (tool, check) in [
        ("write_file", acting(true, &scope)),
        ("shell", shell("ls", &scope)),
    ] {
        assert_eq!(
            gate.evaluate(Origin::Cron, tool, vec![check]).await,
            deny(DenialReason::ScheduledReadOnly),
            "cron must not run {tool}"
        );
    }
    // The same acting call from a live origin is not affected.
    assert_eq!(
        gate.evaluate(Origin::Operator, "write_file", vec![acting(true, &scope)])
            .await,
        Verdict::Allow
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_shell_decision_comes_from_the_registered_command_scope() {
    let mut gate = Gate::start().await;
    let read_only = gate
        .scope(&policy(AutonomyLevel::ReadOnly, true, 100))
        .await;
    assert_eq!(
        gate.evaluate(Origin::Operator, "shell", vec![shell("ls", &read_only)])
            .await,
        Verdict::Allow
    );
    assert_eq!(
        gate.evaluate(
            Origin::Operator,
            "shell",
            vec![shell("touch f", &read_only)]
        )
        .await,
        deny(DenialReason::ReadOnlyMode)
    );
    let supervised = gate
        .scope(&policy(AutonomyLevel::Supervised, true, 100))
        .await;
    assert_eq!(
        gate.evaluate(
            Origin::Operator,
            "shell",
            vec![shell("touch f", &supervised)]
        )
        .await,
        deny(DenialReason::ApprovalUnavailable),
        "a supervised write waits for a human"
    );
    assert_eq!(
        gate.evaluate(
            Origin::Operator,
            "shell",
            vec![shell("echo $(whoami)", &supervised)]
        )
        .await,
        deny(DenialReason::Command),
        "hidden execution is refused"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn shell_runs_and_acting_tools_spend_one_ledger() {
    let mut gate = Gate::start().await;
    let host = policy(AutonomyLevel::Full, true, 2);
    let scope = gate.scope(&host).await;
    // One acting tool spends one action ...
    assert_eq!(
        gate.evaluate(Origin::Operator, "write_file", vec![acting(true, &scope)])
            .await,
        Verdict::Allow
    );
    // ... a shell run (CheckCommand, as the shell tool does) spends the second ...
    let command = host
        .native_command_policy(CommandFlow::Harness, CommandStage::Execute)
        .unwrap();
    let ran = command_on(
        &mut gate.state,
        &gate.fixture.proxy,
        command,
        "ls",
        None,
        CommandOp::Check,
    )
    .await
    .expect("shell run checked");
    assert_eq!(ran.gate, tinysecurity_bus::CommandGate::Allow);
    // ... so the next acting tool sees a spent budget.
    assert_eq!(
        gate.evaluate(Origin::Operator, "write_file", vec![acting(false, &scope)])
            .await,
        deny(DenialReason::RateLimited { max_per_hour: 2 })
    );
}
