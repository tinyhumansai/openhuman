//! A jailed shell and TinySecurity's protected-path floor.

use super::*;

#[cfg(feature = "security-module")]
async fn floor_decision(
    autonomy: AutonomyLevel,
    sandboxed_shell: bool,
    command: &str,
) -> GateDecision {
    let (policy, _) = crate::security::policy::floor_policy(autonomy);
    OpenHumanSecurityGate::new(Arc::new(policy), registry())
        .with_sandboxed_shell(sandboxed_shell)
        .authorize_tool(&req(SHELL_TOOL, json!({ "command": command })))
        .await
        .expect("authorization")
}

/// Production (Nov 2026): a scheduled agent whose shell runs in the kernel
/// jail had `cd ..`, `sed 's/../X/'` and `cat /etc/os-release` refused for
/// naming a protected path, a lexical check the jail makes redundant.
#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_sandboxed_shell_is_not_refused_for_naming_a_protected_path() {
    for command in [
        "cd .. && ls",
        "sed 's/../X/' notes.txt",
        "cat /etc/os-release",
    ] {
        let decision = floor_decision(AutonomyLevel::Full, true, command).await;
        assert_eq!(decision, GateDecision::Allow, "{command}");
    }
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn an_unsandboxed_shell_keeps_the_protected_path_floor() {
    let decision = floor_decision(AutonomyLevel::Full, false, "cd .. && ls").await;
    assert!(!decision.is_allowed(), "{decision:?}");
    assert!(
        format!("{decision:?}").contains("protected path"),
        "{decision:?}"
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_sandboxed_shell_keeps_the_read_only_refusal() {
    let decision = floor_decision(AutonomyLevel::ReadOnly, true, "rm -rf ../build").await;
    assert!(!decision.is_allowed(), "{decision:?}");
    assert!(
        format!("{decision:?}").contains("read-only"),
        "{decision:?}"
    );
}
