use super::*;

// Fails closed only while security is enforced: a build without the
// security module has security disabled (`crate::security::switch`).
#[cfg(feature = "security-module")]
#[tokio::test]
async fn disabled_autonomy_bypasses_class_gating_but_keeps_the_protected_floor() {
    let root = tempfile::tempdir().unwrap();
    let autonomy = crate::config::AutonomyConfig::default();
    assert!(!autonomy.enabled);
    let policy = SecurityPolicy::from_config(&autonomy, root.path(), root.path());

    assert_eq!(
        policy
            .command_class_gate(CommandClass::Destructive)
            .await
            .expect("disabled autonomy must bypass class gating"),
        GateDecision::Allow
    );

    // TinySecurity's protected floor applies whatever the autonomy setting:
    // the module blocks it, and without the module the check refuses.
    let protected = policy
        .check_command(CommandQuery {
            command: "cat ~/.ssh/id_rsa",
            declared: None,
            flow: CommandFlow::Harness,
            stage: CommandStage::Authorize,
        })
        .await;
    assert!(
        !matches!(protected, Ok(ref verdict) if verdict.gate == GateDecision::Allow),
        "a protected path must not be allowed with autonomy disabled: {protected:?}"
    );
}
