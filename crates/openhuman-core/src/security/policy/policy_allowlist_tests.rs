use super::*;
use tinybox_bus::CommandClass;

// -- AutonomyLevel ------------------------------------------------

#[test]
fn autonomy_default_is_supervised() {
    assert_eq!(AutonomyLevel::default(), AutonomyLevel::Supervised);
}

#[test]
fn autonomy_serde_roundtrip() {
    let json = serde_json::to_string(&AutonomyLevel::Full).unwrap();
    assert_eq!(json, "\"full\"");
    let parsed: AutonomyLevel = serde_json::from_str("\"readonly\"").unwrap();
    assert_eq!(parsed, AutonomyLevel::ReadOnly);
    let parsed2: AutonomyLevel = serde_json::from_str("\"supervised\"").unwrap();
    assert_eq!(parsed2, AutonomyLevel::Supervised);
}

#[test]
fn can_act_readonly_false() {
    assert!(!readonly_policy().can_act());
}

#[test]
fn can_act_supervised_true() {
    assert!(default_policy().can_act());
}

#[test]
fn can_act_full_true() {
    assert!(full_policy().can_act());
}

#[test]
fn enforce_tool_operation_read_allowed_in_readonly_mode() {
    let p = readonly_policy();
    assert!(p
        .enforce_tool_operation(ToolOperation::Read, "memory_recall")
        .is_ok());
}

#[test]
fn enforce_tool_operation_act_blocked_in_readonly_mode() {
    let p = readonly_policy();
    let err = p
        .enforce_tool_operation(ToolOperation::Act, "memory_store")
        .unwrap_err();
    assert!(err.contains("read-only mode"));
}

#[test]
fn enforce_tool_operation_act_uses_rate_budget() {
    let p = SecurityPolicy {
        max_actions_per_hour: 0,
        ..default_policy()
    };
    let err = p
        .enforce_tool_operation(ToolOperation::Act, "memory_store")
        .unwrap_err();
    assert!(err.contains("Rate limit exceeded"));
}

#[test]
fn action_budget_error_mentions_limit_and_settings() {
    let p = SecurityPolicy {
        max_actions_per_hour: 0,
        ..default_policy()
    };

    let err = p
        .enforce_tool_operation(ToolOperation::Act, "write_file")
        .unwrap_err();

    assert!(err.contains("Rate limit exceeded: action budget exhausted"));
    assert!(err.contains("0 actions/hour"));
    assert!(err.contains("Settings -> Advanced -> Agent autonomy"));
}

// -- is_command_allowed -------------------------------------------

// -- classify_command / gate_decision (fail-closed bucket model) --

#[test]
fn gate_decision_readonly_blocks_acts() {
    let p = readonly_policy();
    assert_eq!(p.gate_decision(CommandClass::Read), GateDecision::Allow);
    assert_eq!(p.gate_decision(CommandClass::Write), GateDecision::Block);
    assert_eq!(p.gate_decision(CommandClass::Network), GateDecision::Block);
    assert_eq!(
        p.gate_decision(CommandClass::Destructive),
        GateDecision::Block
    );
}

#[test]
fn gate_decision_supervised_prompts_every_act() {
    let p = default_policy(); // Supervised
    assert_eq!(p.gate_decision(CommandClass::Read), GateDecision::Allow);
    assert_eq!(p.gate_decision(CommandClass::Write), GateDecision::Prompt);
    assert_eq!(p.gate_decision(CommandClass::Network), GateDecision::Prompt);
    assert_eq!(
        p.gate_decision(CommandClass::Destructive),
        GateDecision::Prompt
    );
}

#[test]
fn gate_decision_full_runs_write_but_prompts_network_and_destructive() {
    let p = full_policy();
    assert_eq!(p.gate_decision(CommandClass::Read), GateDecision::Allow);
    assert_eq!(p.gate_decision(CommandClass::Write), GateDecision::Allow);
    assert_eq!(p.gate_decision(CommandClass::Network), GateDecision::Prompt);
    assert_eq!(
        p.gate_decision(CommandClass::Destructive),
        GateDecision::Prompt
    );
}

// -- install chokepoint (Phase C) ---------------------------------

#[test]
fn gate_decision_install_always_asks_even_in_full() {
    assert_eq!(
        full_policy().gate_decision(CommandClass::Install),
        GateDecision::Prompt
    );
    assert_eq!(
        default_policy().gate_decision(CommandClass::Install),
        GateDecision::Prompt
    );
    assert_eq!(
        readonly_policy().gate_decision(CommandClass::Install),
        GateDecision::Block
    );
}

// -- cross-platform always-forbidden hardening (Phase E) ----------

#[test]
fn always_forbidden_blocks_credential_stores_case_insensitively() {
    use std::path::Path;
    for p in [
        "/home/u/.ssh/id_rsa",
        "/home/u/.SSH/id_rsa", // case-insensitive
        "C:\\Users\\u\\.ssh\\id_rsa",
        "/home/u/.gnupg/x",
        "/home/u/.aws/credentials",
        "/home/u/.azure/x",
        "/home/u/.kube/config",
        "/Users/u/Library/Keychains/login.keychain",
        "C:\\Users\\u\\AppData\\Roaming\\Microsoft\\Protect\\x",
        "C:\\Users\\u\\AppData\\Local\\Microsoft\\Credentials\\x",
    ] {
        assert!(SecurityPolicy::is_always_forbidden(Path::new(p)), "{p}");
    }
}
