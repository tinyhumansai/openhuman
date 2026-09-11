use super::*;

#[test]
fn default_is_moderate() {
    assert_eq!(AgentActivityLevel::default(), AgentActivityLevel::Moderate);
}

#[test]
fn from_str_round_trips() {
    for level in [
        AgentActivityLevel::Off,
        AgentActivityLevel::Minimal,
        AgentActivityLevel::Moderate,
        AgentActivityLevel::Active,
        AgentActivityLevel::AlwaysOn,
    ] {
        let parsed = AgentActivityLevel::from_str_opt(level.as_str()).unwrap();
        assert_eq!(parsed, level);
    }
}

#[test]
fn serde_repr_round_trips() {
    for level in [
        AgentActivityLevel::Off,
        AgentActivityLevel::Minimal,
        AgentActivityLevel::Moderate,
        AgentActivityLevel::Active,
        AgentActivityLevel::AlwaysOn,
    ] {
        let json = serde_json::to_string(&level).unwrap();
        let parsed: AgentActivityLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, level);
    }
}

#[test]
fn sync_interval_none_for_off() {
    assert_eq!(AgentActivityLevel::Off.sync_interval_secs(), None);
    assert_eq!(
        AgentActivityLevel::Minimal.sync_interval_secs(),
        Some(86_400)
    );
}

#[test]
fn heartbeat_disabled_for_low_levels() {
    assert!(!AgentActivityLevel::Off.heartbeat_enabled());
    assert!(!AgentActivityLevel::Minimal.heartbeat_enabled());
    assert!(AgentActivityLevel::Moderate.heartbeat_enabled());
}
