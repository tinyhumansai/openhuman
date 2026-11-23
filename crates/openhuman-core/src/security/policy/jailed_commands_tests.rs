use super::*;
use crate::security::policy::{
    floor_policy, AutonomyLevel, CommandClass, CommandStage, GateDecision,
};

fn harness(command: &str) -> CommandQuery<'_> {
    CommandQuery {
        command,
        declared: None,
        flow: CommandFlow::Harness,
        stage: CommandStage::Authorize,
    }
}

#[test]
fn neutral_text_replaces_path_words_and_keeps_shell_syntax() {
    assert_eq!(neutral_command_text("cd .. && ls"), "cd x && ls");
    assert_eq!(neutral_command_text("sed 's/../X/' f"), "sed 'x' f");
    assert_eq!(neutral_command_text("cat /etc/os-release"), "cat x");
    assert_eq!(neutral_command_text("cat $HOME/.ssh/id"), "cat $x");
    assert_eq!(
        neutral_command_text("cat $(echo ../x) `ls ../y` 2>&1"),
        "cat $(echo x) `ls x` 2>&1"
    );
    assert_eq!(neutral_command_text("ls -la | wc -l"), "ls -la | wc -l");
}

// Every test below asks a scripted gateway; without the module security is
// disabled (`crate::security::switch`) and nothing is asked at all.

#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_jailed_command_naming_a_protected_path_is_not_refused_for_it() {
    for command in [
        "cd .. && ls",
        "sed 's/../X/' notes.txt",
        "cat /etc/os-release",
    ] {
        let (policy, _) = floor_policy(AutonomyLevel::Full);
        let answer = policy
            .check_jailed_command(harness(command))
            .await
            .unwrap_or_else(|refusal| panic!("{command}: {refusal}"));
        assert!(answer.floor_waived, "{command}");
        assert_eq!(answer.verdict.gate, GateDecision::Allow, "{command}");
    }
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn the_floor_still_refuses_a_command_that_is_not_jailed() {
    let (policy, _) = floor_policy(AutonomyLevel::Full);
    assert_eq!(
        policy.check_command(harness("cd .. && ls")).await,
        Err(Refusal::Protected)
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_jailed_command_keeps_every_other_refusal() {
    // The tier's class gate: read-only mode refuses a destructive command.
    let (policy, _) = floor_policy(AutonomyLevel::ReadOnly);
    assert_eq!(
        policy
            .check_jailed_command(harness("rm -rf ../build"))
            .await,
        Err(Refusal::ReadOnly)
    );
    // The hidden-execution defence.
    let (policy, _) = floor_policy(AutonomyLevel::Supervised);
    assert_eq!(
        policy
            .check_jailed_command(harness("cat $(echo ../secret)"))
            .await,
        Err(Refusal::Hidden)
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_jailed_command_keeps_its_class_and_its_approval() {
    let (policy, gateway) = floor_policy(AutonomyLevel::Supervised);
    let answer = policy
        .check_jailed_command(CommandQuery {
            stage: CommandStage::Execute,
            ..harness("rm -rf ../build")
        })
        .await
        .expect("a jailed destructive command is asked about, not refused");
    assert_eq!(answer.verdict.class, CommandClass::Destructive);
    assert_eq!(answer.verdict.gate, GateDecision::Prompt);
    // The second question carries the original class and the caller's stage,
    // so the module's budget is spent once, at execution.
    let queries = gateway.queries.lock().unwrap();
    assert_eq!(
        queries.last().unwrap(),
        &(
            "rm -rf x".to_owned(),
            Some(CommandClass::Destructive),
            CommandStage::Execute
        )
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_floor_refusal_without_a_known_class_stays_refused() {
    // No path word to neutralize: the floor cannot be asked around.
    let (policy, _) = floor_policy(AutonomyLevel::Full);
    assert_eq!(
        policy
            .check_jailed_command(harness("ls .."))
            .await
            .map(|a| a.floor_waived),
        Ok(true)
    );
    // A gateway that cannot name the class (the default) keeps the refusal.
    #[derive(Debug)]
    struct Classless;
    impl crate::security::policy::Sealed for Classless {}
    #[async_trait::async_trait]
    impl crate::security::policy::CommandGateway for Classless {
        async fn decide(
            &self,
            _policy: &SecurityPolicy,
            _query: &CommandQuery<'_>,
        ) -> Result<CommandVerdict, Refusal> {
            Err(Refusal::Protected)
        }
    }
    let mut policy = SecurityPolicy::default();
    policy.command_gateway = Some(std::sync::Arc::new(Classless));
    assert_eq!(
        policy.check_jailed_command(harness("cd ..")).await,
        Err(Refusal::Protected)
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn scheduled_commands_keep_the_floor() {
    let (policy, _) = floor_policy(AutonomyLevel::Full);
    assert_eq!(
        policy
            .check_jailed_command(CommandQuery {
                flow: CommandFlow::Scheduled,
                ..harness("cd .. && ls")
            })
            .await,
        Err(Refusal::Protected)
    );
}
