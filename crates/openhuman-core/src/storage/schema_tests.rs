use super::*;

#[test]
fn every_owner_composes_without_collision() {
    let registry = registry().expect("the owners compose");
    let owners: Vec<&str> = registry.owners().collect();
    for expected in [
        "harness",
        "session",
        "graph",
        "tasks",
        "approvals",
        "devices",
        "notifications",
        "task_sources",
        "artifacts",
        "cost",
        "config",
        "profiles",
        "threads",
        "subagents",
        "agents",
    ] {
        assert!(owners.contains(&expected), "{expected} missing: {owners:?}");
    }
}

#[test]
fn module_items_are_the_owners_module_scoped_declarations() {
    let mcp = module_items("mcp");
    assert!(mcp
        .collections
        .iter()
        .any(|spec| spec.name == "mcp.servers"));
    assert!(mcp.streams.iter().any(|name| name == "mcp.audit"));
    assert!(module_items("no-such-owner").collections.is_empty());
}
