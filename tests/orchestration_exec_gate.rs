//! Integration coverage for the local-execution trust gate (`exec_gate`).
//!
//! Links the compiled lib (the root crate's `cfg(test)` build is blocked by
//! unrelated stale test modules at this checkout — same reason the pushers are
//! tested from integration tests). Asserts the device-authoritative rule: a
//! local-execution device tool is authorized only for a Master-chat cycle.

use openhuman_core::openhuman::orchestration::exec_gate::{
    cycle_is_master, cycle_target, is_local_execution_tool, record_cycle_origin,
};
use openhuman_core::openhuman::orchestration::types::LOCAL_MASTER_AGENT;

#[test]
fn master_cycle_is_authorized_a2a_and_unknown_are_denied() {
    // A Master-chat forward records the sentinel counterpart + its session.
    record_cycle_origin("gate-cyc-master", LOCAL_MASTER_AGENT, "master");
    // An A2A forward records the peer's real (base58) address + session.
    record_cycle_origin("gate-cyc-peer", "8xPeerBase58AddressExample", "sess-peer");

    assert!(
        cycle_is_master("gate-cyc-master"),
        "master cycle must authorize"
    );
    assert!(
        !cycle_is_master("gate-cyc-peer"),
        "A2A cycle must be denied"
    );
    // Fail closed: a cycle we never recorded (or a forged id) is not master.
    assert!(
        !cycle_is_master("gate-cyc-forged"),
        "unknown cycle must be denied"
    );
    // The completion router resolves the recorded (counterpart, session).
    assert_eq!(
        cycle_target("gate-cyc-master"),
        Some((LOCAL_MASTER_AGENT.to_string(), "master".to_string()))
    );
    assert_eq!(cycle_target("gate-cyc-forged"), None);
}

#[test]
fn only_local_execution_tools_are_gated() {
    assert!(is_local_execution_tool("run_local_agent"));
    assert!(!is_local_execution_tool("device_status"));
}
