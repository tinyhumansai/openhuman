use super::*;

#[test]
fn init_ledger_is_callable_noop() {
    // Must not panic — the boot path calls this unconditionally (itself
    // runtime-gated on `DomainGroup::Web3`) even in a slim build.
    init_ledger(Path::new("/tmp/openhuman-x402-stub-test"), "session-x");
}

#[test]
fn registration_entry_points_are_empty() {
    assert!(all_x402_registered_controllers().is_empty());
    assert!(all_x402_controller_schemas().is_empty());
}
