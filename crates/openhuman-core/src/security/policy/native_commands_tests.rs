use super::super::native_commands_stub::StubGateway;
use super::*;

fn query<'a>(command: &'a str, flow: CommandFlow) -> CommandQuery<'a> {
    CommandQuery {
        command,
        declared: None,
        flow,
        stage: CommandStage::Authorize,
    }
}

#[test]
fn every_refusal_names_the_policy_marker_and_none_echoes_a_command() {
    let all = [
        Refusal::Protected,
        Refusal::ReadOnly,
        Refusal::Hidden,
        Refusal::NotAllowed,
        Refusal::HighRisk,
        Refusal::Category,
        Refusal::Unavailable,
    ];
    for refusal in all {
        let text = refusal.to_string();
        assert!(text.contains(POLICY_BLOCKED_MARKER), "{refusal:?}: {text}");
        assert!(
            text.len() < 400,
            "{refusal:?} message is not a fixed sentence"
        );
    }
    // The rate limit keeps its established wording.
    assert!(Refusal::RateLimit
        .to_string()
        .starts_with("Rate limit exceeded"));
}

#[cfg(feature = "security-module")]
mod projection {
    use super::*;
    use tinysecurity_bus::{
        CommandClass as Bus, CommandDecision, CommandDenial, CommandGate, CommandRisk,
    };

    fn decision(class: Bus, gate: CommandGate, denial: Option<CommandDenial>) -> CommandDecision {
        CommandDecision {
            class,
            risk: CommandRisk::Low,
            gate,
            denial,
        }
    }

    #[test]
    fn an_allowed_or_prompted_command_keeps_its_class_and_gate() {
        let q = query("ls", CommandFlow::Harness);
        assert_eq!(
            project(&q, false, Ok(decision(Bus::Read, CommandGate::Allow, None))),
            Ok(CommandVerdict {
                class: CommandClass::Read,
                gate: GateDecision::Allow
            })
        );
        assert_eq!(
            project(
                &q,
                false,
                Ok(decision(Bus::Install, CommandGate::Prompt, None))
            ),
            Ok(CommandVerdict {
                class: CommandClass::Install,
                gate: GateDecision::Prompt
            })
        );
    }

    #[test]
    fn denials_map_to_the_refusal_for_the_flow_that_asked() {
        let cases = [
            (
                CommandDenial::Protected,
                CommandFlow::Harness,
                Refusal::Protected,
            ),
            (
                CommandDenial::Protected,
                CommandFlow::Scheduled,
                Refusal::Protected,
            ),
            (
                CommandDenial::ReadOnly,
                CommandFlow::Harness,
                Refusal::ReadOnly,
            ),
            (
                CommandDenial::ReadOnly,
                CommandFlow::Scheduled,
                Refusal::NotAllowed,
            ),
            (
                CommandDenial::Command,
                CommandFlow::Harness,
                Refusal::Hidden,
            ),
            (
                CommandDenial::Command,
                CommandFlow::Scheduled,
                Refusal::NotAllowed,
            ),
            (
                CommandDenial::HighRisk,
                CommandFlow::Harness,
                Refusal::HighRisk,
            ),
            (
                CommandDenial::RateLimit,
                CommandFlow::Scheduled,
                Refusal::RateLimit,
            ),
        ];
        for (denial, flow, expected) in cases {
            let answer = decision(Bus::Write, CommandGate::Block, Some(denial));
            assert_eq!(
                project(&query("x", flow), false, Ok(answer)),
                Err(expected),
                "{denial:?} {flow:?}"
            );
        }
    }

    #[test]
    fn a_denial_wins_even_if_the_gate_says_allow() {
        let answer = decision(
            Bus::Read,
            CommandGate::Allow,
            Some(CommandDenial::Protected),
        );
        assert_eq!(
            project(&query("x", CommandFlow::Harness), false, Ok(answer)),
            Err(Refusal::Protected)
        );
    }

    #[test]
    fn a_block_without_a_reason_is_a_refusal_for_a_command_and_a_block_for_a_class() {
        let answer = decision(Bus::Network, CommandGate::Block, None);
        assert_eq!(
            project(
                &query("curl x", CommandFlow::Harness),
                false,
                Ok(answer.clone())
            ),
            Err(Refusal::Category)
        );
        assert_eq!(
            project(&query("", CommandFlow::Harness), true, Ok(answer)),
            Ok(CommandVerdict {
                class: CommandClass::Network,
                gate: GateDecision::Block
            })
        );
    }

    #[test]
    fn a_module_that_cannot_answer_never_becomes_an_allow() {
        for class_only in [false, true] {
            assert_eq!(
                project(
                    &query("ls", CommandFlow::Harness),
                    class_only,
                    Err("TinySecurity bus unavailable".into())
                ),
                Err(Refusal::Unavailable)
            );
        }
    }

    #[test]
    fn every_module_gate_has_a_host_gate() {
        assert_eq!(host_gate(CommandGate::Allow), GateDecision::Allow);
        assert_eq!(host_gate(CommandGate::Prompt), GateDecision::Prompt);
        assert_eq!(host_gate(CommandGate::Block), GateDecision::Block);
    }

    /// The security gate asks the module about acting tools under this scope, so the
    /// scope must carry what makes the budget and the tier inert while the autonomy
    /// policy is disabled, and the real limit and tier while it is enabled. The execute
    /// stage is the one the shell tool spends its budget through, which acting tools
    /// share.
    #[test]
    fn the_execute_scope_carries_the_autonomy_switch_tier_and_budget() {
        let scope = |enabled, autonomy, budget| {
            SecurityPolicy {
                enabled,
                autonomy,
                max_actions_per_hour: budget,
                ..SecurityPolicy::default()
            }
            .native_command_policy(CommandFlow::Harness, CommandStage::Execute)
            .unwrap()
        };
        let disabled = scope(false, crate::security::AutonomyLevel::ReadOnly, 1);
        assert!(!disabled.enabled);
        assert_eq!(disabled.max_actions_per_hour, 1);
        let enabled = scope(true, crate::security::AutonomyLevel::ReadOnly, 7);
        assert!(enabled.enabled);
        assert_eq!(
            enabled.autonomy,
            tinysecurity_bus::CommandAutonomy::ReadOnly
        );
        assert_eq!(enabled.max_actions_per_hour, 7);
        // The authorize stage never spends the budget the run reserves.
        let authorize = SecurityPolicy {
            max_actions_per_hour: 7,
            ..SecurityPolicy::default()
        }
        .native_command_policy(CommandFlow::Harness, CommandStage::Authorize)
        .unwrap();
        assert_eq!(authorize.max_actions_per_hour, u32::MAX);
    }

    #[test]
    fn a_relative_action_dir_becomes_absolute_and_a_missing_home_stays_none_or_absolute() {
        let policy = SecurityPolicy {
            action_dir: std::path::PathBuf::from("relative/dir"),
            ..SecurityPolicy::default()
        };
        let scope = policy
            .native_command_policy(CommandFlow::Harness, CommandStage::Execute)
            .unwrap();
        assert!(scope.action_dir.is_absolute());
        assert!(scope.action_dir.ends_with("relative/dir"));
        assert!(scope.home_dir.is_none_or(|home| home.is_absolute()));
    }
}

#[tokio::test]
async fn an_installed_gateway_answers_and_its_clones_keep_it() {
    let gateway = StubGateway::prompt();
    let policy = SecurityPolicy::default().with_gateway(gateway.clone());
    let verdict = policy
        .clone()
        .check_command(query("touch f", CommandFlow::Harness))
        .await
        .unwrap();
    assert_eq!(verdict.gate, GateDecision::Prompt);
    assert_eq!(gateway.asked().len(), 1);
}

#[tokio::test]
async fn class_gates_ask_the_gateway_with_no_command_text() {
    let gateway = StubGateway::gate(CommandClass::Network, GateDecision::Prompt);
    let policy = SecurityPolicy::default().with_gateway(gateway.clone());
    assert_eq!(
        policy.command_class_gate(CommandClass::Network).await,
        Ok(GateDecision::Prompt)
    );
    assert_eq!(
        policy.command_class_gate_blocking(CommandClass::Network),
        Ok(GateDecision::Prompt)
    );
    let asked = gateway.asked();
    assert_eq!(asked.len(), 2);
    for question in asked {
        assert!(question.command.is_empty());
        assert_eq!(question.declared, Some(CommandClass::Network));
    }
}

#[test]
fn the_blocking_class_gate_works_outside_any_runtime() {
    let policy = SecurityPolicy::default().with_gateway(StubGateway::allow());
    assert_eq!(
        policy.command_class_gate_blocking(CommandClass::Write),
        Ok(GateDecision::Allow)
    );
}

/// With no gateway the module is the only authority: a module that is disabled
/// (or cannot load) refuses every command, with no local classifier to fall back
/// on. The production client latches faults for the whole process, so this runs
/// in a child process, as the path-validation twin does.
#[tokio::test]
async fn without_the_module_every_command_and_class_gate_is_refused() {
    const CHILD: &str = "OPENHUMAN_NATIVE_COMMAND_UNAVAILABLE_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "security::policy::native_commands::tests::without_the_module_every_command_and_class_gate_is_refused",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let action = dir.path().join("project");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&action).unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[modules]\nenabled = false\n",
    )
    .unwrap();
    let policy = SecurityPolicy {
        enabled: false,
        workspace_dir: workspace,
        action_dir: action,
        ..SecurityPolicy::default()
    };
    // The most permissive policy there is still refuses without the module.
    for command in ["ls", "echo hello", ""] {
        let refusal = policy
            .check_command(query(command, CommandFlow::Harness))
            .await
            .unwrap_err();
        assert_eq!(refusal, Refusal::Unavailable, "{command:?}");
    }
    assert_eq!(
        policy
            .check_command(CommandQuery {
                stage: CommandStage::Execute,
                ..query("ls", CommandFlow::Scheduled)
            })
            .await,
        Err(Refusal::Unavailable)
    );
    assert_eq!(
        policy.command_class_gate(CommandClass::Read).await,
        Err(Refusal::Unavailable)
    );
}
