use super::*;
use proptest::prelude::*;

fn class() -> impl Strategy<Value = tinysecurity_bus::CommandClass> {
    use tinysecurity_bus::CommandClass;
    prop_oneof![
        Just(CommandClass::Read),
        Just(CommandClass::Write),
        Just(CommandClass::Network),
        Just(CommandClass::Install),
        Just(CommandClass::Destructive),
    ]
}

fn gate() -> impl Strategy<Value = tinysecurity_bus::CommandGate> {
    use tinysecurity_bus::CommandGate;
    prop_oneof![
        Just(CommandGate::Allow),
        Just(CommandGate::Prompt),
        Just(CommandGate::Block),
    ]
}

fn denial() -> impl Strategy<Value = Option<tinysecurity_bus::CommandDenial>> {
    use tinysecurity_bus::CommandDenial;
    prop_oneof![
        Just(None),
        Just(Some(CommandDenial::Protected)),
        Just(Some(CommandDenial::ReadOnly)),
        Just(Some(CommandDenial::Command)),
        Just(Some(CommandDenial::HighRisk)),
        Just(Some(CommandDenial::RateLimit)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn projection_refuses_every_denial_and_never_echoes_the_command(
        class in class(),
        gate in gate(),
        denial in denial(),
        scheduled in any::<bool>(),
        class_only in any::<bool>(),
        command in "[ -~]{8,40}",
    ) {
        let query = CommandQuery {
            command: &command,
            declared: None,
            flow: if scheduled { CommandFlow::Scheduled } else { CommandFlow::Harness },
            stage: CommandStage::Execute,
        };
        let answer = tinysecurity_bus::CommandDecision {
            class,
            risk: tinysecurity_bus::CommandRisk::Low,
            gate,
            denial,
        };
        let denied = denial.is_some();
        let blocked_real_command = gate == tinysecurity_bus::CommandGate::Block && !class_only;
        let verdict = project(&query, class_only, Ok(answer));
        if denied || blocked_real_command {
            prop_assert!(verdict.is_err());
        }
        if let Err(refusal) = verdict {
            prop_assert!(!refusal.to_string().contains(&command));
        }
    }

    #[test]
    fn an_unanswered_question_is_always_refused(
        command in ".*",
        class_only in any::<bool>(),
    ) {
        let query = CommandQuery {
            command: &command,
            declared: None,
            flow: CommandFlow::Harness,
            stage: CommandStage::Authorize,
        };
        prop_assert_eq!(
            project(&query, class_only, Err(command.clone())),
            Err(Refusal::Unavailable),
        );
    }

    #[test]
    fn policy_translation_never_panics(
        allowed in proptest::collection::vec(".*", 0..8),
        budget in any::<u32>(),
        enabled in any::<bool>(),
    ) {
        let policy = SecurityPolicy {
            enabled,
            autonomy: AutonomyLevel::Supervised,
            allowed_commands: allowed,
            max_actions_per_hour: budget,
            ..SecurityPolicy::default()
        };
        let authorize = policy
            .native_command_policy(CommandFlow::Harness, CommandStage::Authorize)
            .expect("an absolute or resolvable action dir translates");
        prop_assert_eq!(authorize.max_actions_per_hour, u32::MAX);
        let execute = policy
            .native_command_policy(CommandFlow::Scheduled, CommandStage::Execute)
            .expect("translates");
        prop_assert_eq!(execute.max_actions_per_hour, budget);
    }
}
