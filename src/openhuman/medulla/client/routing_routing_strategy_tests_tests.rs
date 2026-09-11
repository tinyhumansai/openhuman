use super::RoutingStrategy;

#[test]
fn wire_tokens_round_trip() {
    for s in [
        RoutingStrategy::Manual,
        RoutingStrategy::Balanced,
        RoutingStrategy::CpuFirst,
        RoutingStrategy::MemoryFirst,
    ] {
        assert_eq!(RoutingStrategy::from_wire(s.as_wire()), Some(s));
    }
}

#[test]
fn unknown_token_is_none_not_an_error() {
    assert_eq!(RoutingStrategy::from_wire("quantumFirst"), None);
    assert_eq!(RoutingStrategy::from_wire(""), None);
}
