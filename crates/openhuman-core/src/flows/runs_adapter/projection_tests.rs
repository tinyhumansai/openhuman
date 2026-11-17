use super::*;

#[test]
fn catalog_flow_triggers_map_to_the_serde_bus_contract() {
    let cases = [
        (FlowRunTrigger::Rpc, runs::FlowRunTrigger::Rpc),
        (FlowRunTrigger::Schedule, runs::FlowRunTrigger::Schedule),
        (FlowRunTrigger::AppEvent, runs::FlowRunTrigger::AppEvent),
        (FlowRunTrigger::Resume, runs::FlowRunTrigger::Resume),
    ];
    for (catalog, bus) in cases {
        assert_eq!(bus_trigger(catalog), bus);
    }
}
