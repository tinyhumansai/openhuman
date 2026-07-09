use super::*;
use crate::openhuman::config::schema::SubconsciousMode;
use crate::openhuman::config::Config;

#[test]
fn kind_id_and_parse_round_trip() {
    for kind in SubconsciousKind::ALL {
        assert_eq!(SubconsciousKind::parse(kind.id()), Some(kind));
    }
    assert_eq!(SubconsciousKind::parse("nope"), None);
}

#[test]
fn enabled_kinds_gates_memory_on_heartbeat_and_mode() {
    let mut cfg = Config::default();

    // Heartbeat enabled + an enabled mode + orchestration on (default) → both.
    cfg.heartbeat.enabled = true;
    cfg.heartbeat.subconscious_mode = SubconsciousMode::Simple;
    cfg.orchestration.enabled = true;
    assert_eq!(
        SubconsciousKind::enabled_kinds(&cfg),
        vec![SubconsciousKind::Memory, SubconsciousKind::TinyPlace]
    );

    // Mode Off drops memory but tinyplace still runs on orchestration.enabled.
    cfg.heartbeat.subconscious_mode = SubconsciousMode::Off;
    assert_eq!(
        SubconsciousKind::enabled_kinds(&cfg),
        vec![SubconsciousKind::TinyPlace]
    );

    // Heartbeat disabled drops memory regardless of mode.
    cfg.heartbeat.enabled = false;
    cfg.heartbeat.subconscious_mode = SubconsciousMode::Aggressive;
    assert_eq!(
        SubconsciousKind::enabled_kinds(&cfg),
        vec![SubconsciousKind::TinyPlace]
    );

    // Orchestration off too → nothing runs.
    cfg.orchestration.enabled = false;
    assert!(SubconsciousKind::enabled_kinds(&cfg).is_empty());
}

#[test]
fn make_subconscious_builds_each_kind_with_matching_id() {
    let cfg = Config::default();
    assert_eq!(
        make_subconscious(SubconsciousKind::Memory, &cfg).id(),
        "memory"
    );
    assert_eq!(
        make_subconscious(SubconsciousKind::TinyPlace, &cfg).id(),
        "tinyplace"
    );
}
