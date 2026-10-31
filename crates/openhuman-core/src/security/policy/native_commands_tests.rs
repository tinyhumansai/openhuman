use super::*;

#[tokio::test]
async fn disabled_autonomy_does_not_require_tinysecurity_for_commands() {
    let root = tempfile::tempdir().unwrap();
    let autonomy = crate::config::AutonomyConfig::default();
    assert!(!autonomy.enabled);
    let policy = SecurityPolicy::from_config(&autonomy, root.path(), root.path());

    let command = policy
        .check_command(CommandQuery {
            command: "echo test",
            declared: None,
            flow: CommandFlow::Harness,
            stage: CommandStage::Authorize,
        })
        .await
        .expect("disabled autonomy must not consult TinySecurity");
    assert_eq!(command.gate, GateDecision::Allow);
    assert_eq!(command.class, CommandClass::Write);

    assert_eq!(
        policy
            .command_class_gate(CommandClass::Destructive)
            .await
            .expect("disabled autonomy must bypass class gating"),
        GateDecision::Allow
    );
}
