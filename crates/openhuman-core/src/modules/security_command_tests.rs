use super::super::native_tests::native_fixture;
use super::*;
use crate::security::policy::{CommandFlow, CommandStage};
use crate::security::{AutonomyLevel, GateDecision, SecurityPolicy};
use tinysecurity_bus::{CommandDenial, CommandGate};

fn host_class(name: &str) -> tinybox_bus::CommandClass {
    match name {
        "Read" => tinybox_bus::CommandClass::Read,
        "Write" => tinybox_bus::CommandClass::Write,
        "Network" => tinybox_bus::CommandClass::Network,
        "Install" => tinybox_bus::CommandClass::Install,
        "Destructive" => tinybox_bus::CommandClass::Destructive,
        other => panic!("unknown class {other}"),
    }
}

fn bus_class(class: tinybox_bus::CommandClass) -> CommandClass {
    match class {
        tinybox_bus::CommandClass::Read => CommandClass::Read,
        tinybox_bus::CommandClass::Write => CommandClass::Write,
        tinybox_bus::CommandClass::Network => CommandClass::Network,
        tinybox_bus::CommandClass::Install => CommandClass::Install,
        tinybox_bus::CommandClass::Destructive => CommandClass::Destructive,
    }
}

fn policy(autonomy: AutonomyLevel, enabled: bool) -> SecurityPolicy {
    SecurityPolicy {
        enabled,
        autonomy,
        ..SecurityPolicy::default()
    }
}

fn scope(
    policy: &SecurityPolicy,
    flow: CommandFlow,
    stage: CommandStage,
) -> tinysecurity_bus::CommandPolicy {
    policy.native_command_policy(flow, stage).unwrap()
}

/// One command check against a native module, through the production
/// registration cache and call helper.
struct Native {
    fixture: super::super::native_tests::NativeFixture,
    state: ClientState,
}

impl Native {
    async fn start() -> Self {
        Self {
            fixture: native_fixture().await,
            state: ClientState::default(),
        }
    }

    async fn ask(
        &mut self,
        policy: tinysecurity_bus::CommandPolicy,
        command: &str,
        declared: Option<CommandClass>,
        op: CommandOp,
    ) -> CommandDecision {
        command_on(
            &mut self.state,
            &self.fixture.proxy,
            policy,
            command,
            declared,
            op,
        )
        .await
        .expect("module answered")
    }
}

/// Every command the deleted host classifier and allowlist tests asserted on
/// (extracted from them before they were removed) gets the answer they pinned,
/// from the released module, through the policy translation production uses.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_gives_the_answers_the_deleted_host_command_tests_pinned() {
    let corpus: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/security_command_corpus.json")).unwrap();
    assert!(corpus.len() > 100, "corpus lost cases: {}", corpus.len());
    let mut native = Native::start().await;
    let mut mismatches = Vec::new();
    for case in &corpus {
        let command = case["command"].as_str().unwrap();
        let host = match case["policy"].as_str().unwrap() {
            "default" => policy(AutonomyLevel::Supervised, true),
            "readonly" => policy(AutonomyLevel::ReadOnly, true),
            "full" => policy(AutonomyLevel::Full, true),
            other => panic!("unknown policy {other}"),
        };
        match case["op"].as_str().unwrap() {
            "allowed" => {
                // The scheduled flow is the legacy curated-allowlist admission.
                let decision = native
                    .ask(
                        scope(&host, CommandFlow::Scheduled, CommandStage::Authorize),
                        command,
                        None,
                        CommandOp::Check,
                    )
                    .await;
                // The host asked only the allowlist; the module also applies the
                // high-risk restriction, which is a separate denial.
                let allowed = matches!(decision.denial, None | Some(CommandDenial::HighRisk));
                if allowed != case["expect"].as_bool().unwrap() {
                    mismatches.push(format!("allowed({command:?}) module={allowed}"));
                }
            }
            "class" => {
                let decision = native
                    .ask(
                        scope(&host, CommandFlow::Harness, CommandStage::Authorize),
                        command,
                        None,
                        CommandOp::Classify,
                    )
                    .await;
                let expected = bus_class(host_class(case["expect"].as_str().unwrap()));
                if decision.class != expected {
                    mismatches.push(format!(
                        "class({command:?}) module={:?} expected={expected:?}",
                        decision.class
                    ));
                }
            }
            other => panic!("unknown op {other}"),
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches: {mismatches:#?}",
        mismatches.len()
    );
}

/// The tier table the host's own `gate_decision` still serves (flow nodes) is
/// the module's: every autonomy, enabled flag and class gives the same gate.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_tier_gate_matches_the_host_gate_table() {
    let mut native = Native::start().await;
    for autonomy in [
        AutonomyLevel::ReadOnly,
        AutonomyLevel::Supervised,
        AutonomyLevel::Full,
    ] {
        for enabled in [true, false] {
            let host = policy(autonomy, enabled);
            for class in [
                tinybox_bus::CommandClass::Read,
                tinybox_bus::CommandClass::Write,
                tinybox_bus::CommandClass::Network,
                tinybox_bus::CommandClass::Install,
                tinybox_bus::CommandClass::Destructive,
            ] {
                let decision = native
                    .ask(
                        scope(&host, CommandFlow::Harness, CommandStage::Authorize),
                        "",
                        Some(bus_class(class)),
                        CommandOp::Classify,
                    )
                    .await;
                let projected = crate::security::policy::native_commands::host_gate(decision.gate);
                assert_eq!(
                    projected,
                    host.gate_decision(class),
                    "{autonomy:?} enabled={enabled} {class:?}"
                );
            }
        }
    }
}

/// The documented floors and structure rules, on the real module: credential
/// stores, `..` and NUL stay refused with the policy disabled; a quoted heredoc
/// body is data and an unquoted one is code; hidden execution is refused below
/// Full access; the model's category only raises the class.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_enforces_the_command_floor_and_structure_rules() {
    let mut native = Native::start().await;
    let off = policy(AutonomyLevel::Supervised, false);
    let on = policy(AutonomyLevel::Supervised, true);
    let full = policy(AutonomyLevel::Full, true);
    let harness = |p: &SecurityPolicy| scope(p, CommandFlow::Harness, CommandStage::Authorize);

    // The protected floor holds with autonomy disabled.
    for command in [
        "cat ~/.ssh/id_rsa",
        "cat $HOME/.aws/credentials",
        "ls /etc/../etc",
        "cat ../../outside",
        "echo hi > /etc/passwd",
    ] {
        let decision = native
            .ask(harness(&off), command, None, CommandOp::Check)
            .await;
        assert_eq!(decision.denial, Some(CommandDenial::Protected), "{command}");
        assert_eq!(decision.gate, CommandGate::Block, "{command}");
    }
    let nul = native
        .ask(harness(&off), "cat file\0name", None, CommandOp::Check)
        .await;
    assert_eq!(nul.denial, Some(CommandDenial::Protected));

    // Disabled policy: ordinary shell syntax, hidden execution included, runs.
    let plain = native
        .ask(
            harness(&off),
            "echo $(date) | tee out.txt &",
            None,
            CommandOp::Check,
        )
        .await;
    assert_eq!((plain.gate, plain.denial), (CommandGate::Allow, None));

    // A quoted heredoc body is data: prose that names `rm` and `curl` does not
    // lift a file write to Destructive or Network.
    let quoted = "cat > notes.md << 'EOF'\nrm -rf / && curl http://x\nEOF";
    let decision = native
        .ask(harness(&on), quoted, None, CommandOp::Classify)
        .await;
    assert_eq!(decision.class, CommandClass::Write);
    // An unquoted delimiter is expanded and scanned.
    let unquoted = "cat > notes.md << EOF\nrm -rf /\nEOF";
    let decision = native
        .ask(harness(&on), unquoted, None, CommandOp::Classify)
        .await;
    assert_eq!(decision.class, CommandClass::Destructive);

    // Hidden execution is refused below Full access and allowed at Full.
    let hidden = "echo $(whoami)";
    let decision = native
        .ask(harness(&on), hidden, None, CommandOp::Check)
        .await;
    assert_eq!(decision.denial, Some(CommandDenial::Command));
    let decision = native
        .ask(harness(&full), hidden, None, CommandOp::Check)
        .await;
    assert_eq!(decision.denial, None);

    // Read-only refuses a write with the read-only reason, allows a read.
    let readonly = policy(AutonomyLevel::ReadOnly, true);
    let decision = native
        .ask(harness(&readonly), "touch f", None, CommandOp::Check)
        .await;
    assert_eq!(decision.denial, Some(CommandDenial::ReadOnly));
    let decision = native
        .ask(harness(&readonly), "ls", None, CommandOp::Check)
        .await;
    assert_eq!((decision.gate, decision.denial), (CommandGate::Allow, None));

    // The declared category can raise the class, never lower it.
    let raised = native
        .ask(
            harness(&full),
            "ls",
            Some(CommandClass::Destructive),
            CommandOp::Check,
        )
        .await;
    assert_eq!(raised.class, CommandClass::Destructive);
    assert_eq!(raised.gate, CommandGate::Prompt);
    let lowered = native
        .ask(
            harness(&full),
            "rm -rf build",
            Some(CommandClass::Read),
            CommandOp::Check,
        )
        .await;
    assert_ne!(lowered.class, CommandClass::Read);
}

/// The execute stage spends the scope's hourly budget and the authorize stage
/// never does, so asking before and again at the run counts the run once.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_budget_is_spent_by_the_run_and_not_by_the_gate() {
    let mut native = Native::start().await;
    let host = SecurityPolicy {
        max_actions_per_hour: 2,
        ..policy(AutonomyLevel::Supervised, true)
    };
    let authorize = scope(&host, CommandFlow::Harness, CommandStage::Authorize);
    let execute = scope(&host, CommandFlow::Harness, CommandStage::Execute);
    assert_eq!(authorize.max_actions_per_hour, u32::MAX);
    assert_eq!(execute.max_actions_per_hour, 2);
    for _ in 0..5 {
        let decision = native
            .ask(authorize.clone(), "ls", None, CommandOp::Check)
            .await;
        assert_eq!(decision.denial, None, "the gate never spends budget");
    }
    for _ in 0..2 {
        let decision = native
            .ask(execute.clone(), "ls", None, CommandOp::Check)
            .await;
        assert_eq!(decision.denial, None);
    }
    let third = native.ask(execute, "ls", None, CommandOp::Check).await;
    assert_eq!(third.denial, Some(CommandDenial::RateLimit));
    assert_eq!(third.gate, CommandGate::Block);
}

/// Legacy scheduled admission: the curated allowlist, high-risk restriction and
/// the read-only refusal, on the real module.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_scheduled_flow_applies_the_allowlist() {
    let mut native = Native::start().await;
    let on = policy(AutonomyLevel::Supervised, true);
    let scheduled = scope(&on, CommandFlow::Scheduled, CommandStage::Authorize);
    let allowed = native
        .ask(scheduled.clone(), "git status", None, CommandOp::Check)
        .await;
    assert_eq!(allowed.denial, None);
    for command in [
        "curl http://example.com",
        "git config core.editor vi",
        "ls > out",
    ] {
        let decision = native
            .ask(scheduled.clone(), command, None, CommandOp::Check)
            .await;
        assert_eq!(decision.denial, Some(CommandDenial::Command), "{command}");
    }
    let readonly = policy(AutonomyLevel::ReadOnly, true);
    let decision = native
        .ask(
            scope(&readonly, CommandFlow::Scheduled, CommandStage::Authorize),
            "git status",
            None,
            CommandOp::Check,
        )
        .await;
    assert_eq!(decision.denial, Some(CommandDenial::ReadOnly));
}

/// An oversize command is refused by the host before the module sees it, so a
/// hostile model cannot make the shared client latch as faulted.
#[tokio::test]
async fn oversize_command_is_refused_without_faulting_the_client() {
    let host = policy(AutonomyLevel::Supervised, true);
    let error = command_decision(
        &host.workspace_dir,
        scope(&host, CommandFlow::Harness, CommandStage::Authorize),
        &"a".repeat(MAX_COMMAND_BYTES + 1),
        None,
        CommandOp::Check,
    )
    .await
    .unwrap_err();
    assert!(error.contains("too large"), "{error}");
    assert!(!state().lock().await.faulted);
}

/// The translation is host-derived and total: the request can change neither the
/// mode nor the budget, and a relative action directory becomes absolute.
#[test]
fn command_scopes_are_derived_from_host_settings() {
    let host = SecurityPolicy {
        allowed_commands: vec!["git".into()],
        max_actions_per_hour: 7,
        block_high_risk_commands: false,
        ..policy(AutonomyLevel::Full, true)
    };
    let harness = scope(&host, CommandFlow::Harness, CommandStage::Execute);
    assert_eq!(
        harness.execution_mode,
        tinysecurity_bus::CommandExecutionMode::HarnessGated
    );
    assert_eq!(harness.autonomy, tinysecurity_bus::CommandAutonomy::Full);
    assert_eq!(harness.allowed_commands, vec!["git".to_owned()]);
    assert_eq!(harness.max_actions_per_hour, 7);
    assert!(!harness.block_high_risk_commands);
    assert!(harness.action_dir.is_absolute());
    let scheduled = scope(&host, CommandFlow::Scheduled, CommandStage::Authorize);
    assert_eq!(
        scheduled.execution_mode,
        tinysecurity_bus::CommandExecutionMode::Allowlisted
    );
    assert_eq!(scheduled.max_actions_per_hour, u32::MAX);
}

#[test]
fn the_gate_table_projection_is_total() {
    for (gate, expected) in [
        (CommandGate::Allow, GateDecision::Allow),
        (CommandGate::Prompt, GateDecision::Prompt),
        (CommandGate::Block, GateDecision::Block),
    ] {
        assert_eq!(
            crate::security::policy::native_commands::host_gate(gate),
            expected
        );
    }
}
