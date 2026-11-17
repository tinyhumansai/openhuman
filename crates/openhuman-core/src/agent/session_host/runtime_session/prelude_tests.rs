use super::*;
use std::{collections::HashSet, sync::Arc};
use tinyagents_harness::tool::ToolSnapshot;
use tinytools::ToolSpec;

fn spec(name: &str, description: &str) -> Arc<ToolSpec> {
    Arc::new(ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters: serde_json::json!({"type": "object"}),
    })
}

#[test]
fn a_deferred_host_tool_shadowing_a_builtin_is_granted_once_as_the_host_tool() {
    // `HostTurnTools::merge_into` splices host tools ahead of the
    // config-derived belt, so the registry holds the host's `file_read`
    // first and the built-in second.
    let registered = vec![
        spec("file_read", "host: read a project file"),
        spec("shell", "built-in shell"),
        spec("file_read", "built-in: read any file"),
    ];
    let visible = vec![spec("shell", "built-in shell")];
    let deferred: HashSet<String> = ["file_read".to_string()].into();

    let granted = granted_tool_specs(&visible, &registered, &deferred, false);

    let reads: Vec<_> = granted.iter().filter(|s| s.name == "file_read").collect();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].description, "host: read a project file");
    let snapshot = ToolSnapshot::new(granted).expect("name-unique declarations freeze");
    assert_eq!(snapshot.specs().len(), 2);
}

#[test]
fn a_tool_both_advertised_and_deferred_keeps_the_advertised_declaration() {
    let registered = vec![spec("lookup", "deferred copy")];
    let visible = vec![spec("lookup", "advertised copy")];
    let deferred: HashSet<String> = ["lookup".to_string()].into();

    let granted = granted_tool_specs(&visible, &registered, &deferred, true);

    assert_eq!(granted.len(), 1);
    assert_eq!(granted[0].description, "advertised copy");
}

/// With pruning on, a recorded declaration survives only while the snapshot
/// offers its name. A deferred tool the host has not rebuilt yet has no
/// declaration to grant, but it is still offered through the catalogue.
#[test]
fn the_granted_snapshot_offers_every_deferred_name_even_without_a_declaration() {
    let visible = vec![spec("shell", "built-in shell")];
    let registered = vec![spec("lookup", "deferred copy")];
    let deferred: HashSet<String> = ["lookup".to_string(), "not_rebuilt_yet".to_string()].into();

    let snapshot = granted_tool_snapshot(&visible, &registered, &deferred, true).expect("freezes");

    assert!(snapshot.offers("shell"));
    assert!(snapshot.offers("lookup"));
    assert!(snapshot.offers("not_rebuilt_yet"));
    assert!(!snapshot.offers("retired_tool"));
    assert!(snapshot
        .specs()
        .iter()
        .all(|spec| spec.name != "not_rebuilt_yet"));

    let recorded = ToolSnapshot::new(vec![
        (*spec("not_rebuilt_yet", "recorded")).clone(),
        (*spec("retired_tool", "recorded")).clone(),
    ])
    .unwrap();
    let (merged, retained) = snapshot.with_retained_offered(&recorded).unwrap();
    assert_eq!(retained, 1, "only the still-offered declaration is kept");
    assert!(merged
        .specs()
        .iter()
        .any(|spec| spec.name == "not_rebuilt_yet"));
    assert!(merged
        .specs()
        .iter()
        .all(|spec| spec.name != "retired_tool"));
}
