//! Tests for the flow → catalogue-entry mapping.

use super::*;
use tinyflows::model::{Node, NodeKind, WorkflowGraph};
use tinyflows_catalog::types::Flow;

fn node(id: &str, kind: NodeKind, config: serde_json::Value) -> Node {
    Node {
        id: id.into(),
        kind,
        name: id.to_string(),
        config,
        type_version: 1,
        ports: Vec::new(),
        position: None,
    }
}

fn flow(name: &str, enabled: bool, nodes: Vec<Node>) -> Flow {
    Flow {
        id: "flow-abc-123".to_string(),
        name: name.to_string(),
        enabled,
        graph: WorkflowGraph {
            nodes,
            ..Default::default()
        },
        created_at: String::new(),
        updated_at: String::new(),
        last_run_at: None,
        last_status: None,
        require_approval: false,
    }
}

#[test]
fn the_entry_is_keyed_by_the_flow_id_not_a_slug_of_its_name() {
    // `run_workflow` and `get_flow` take the id. A slug of the display name
    // would be a second identifier that resolves nowhere, and the model would
    // have no way to tell which one it was holding.
    let entry = entry_for(flow("Morning Digest", true, vec![]));
    assert_eq!(entry.dir_name, "flow-abc-123");
    assert_eq!(entry.name, "Morning Digest");
}

#[test]
fn the_entry_is_scoped_flow_and_carries_no_on_disk_bundle() {
    // The distinction that makes `WorkflowScope::Flow` worth having: consumers
    // that read `location` or `resources` must get an honest absence rather
    // than a path that does not exist.
    let entry = entry_for(flow("Anything", true, vec![]));
    assert_eq!(entry.scope, WorkflowScope::Flow);
    assert!(entry.location.is_none());
    assert!(entry.resources.is_empty());
}

#[test]
fn the_description_names_the_trigger_and_step_count() {
    let f = flow(
        "Digest",
        true,
        vec![
            node(
                "t",
                NodeKind::Trigger,
                serde_json::json!({ "trigger_kind": "schedule" }),
            ),
            node("a", NodeKind::Agent, serde_json::json!({})),
            node("b", NodeKind::Agent, serde_json::json!({})),
        ],
    );
    let entry = entry_for(f);
    assert!(
        entry.description.contains("schedule"),
        "{}",
        entry.description
    );
    // Two steps: the trigger is not a step a user thinks about.
    assert!(
        entry.description.contains("2 steps"),
        "{}",
        entry.description
    );
}

#[test]
fn a_single_step_is_not_pluralised() {
    let f = flow(
        "One",
        true,
        vec![
            node(
                "t",
                NodeKind::Trigger,
                serde_json::json!({ "trigger_kind": "manual" }),
            ),
            node("a", NodeKind::Agent, serde_json::json!({})),
        ],
    );
    assert!(entry_for(f).description.contains("1 step)"));
}

#[test]
fn a_graph_with_no_trigger_reads_as_manual_rather_than_blank() {
    // `graph.trigger()` also returns `None` when there are *several* triggers.
    // Either way the catalogue must say something; validation reports the real
    // problem separately, so this stays quiet rather than duplicating it.
    let entry = entry_for(flow("Headless", true, vec![]));
    assert!(
        entry.description.contains("manual"),
        "{}",
        entry.description
    );
}

#[test]
fn a_disabled_flow_is_still_listed_and_says_it_is_paused() {
    // Hiding it would make the model answer "you have no such automation" to
    // someone looking straight at it in the UI.
    let entry = entry_for(flow("Paused", false, vec![]));
    assert!(entry.description.contains("Currently disabled"));
}

#[test]
fn an_enabled_flow_does_not_claim_to_be_disabled() {
    assert!(!entry_for(flow("Live", true, vec![]))
        .description
        .contains("disabled"));
}

#[test]
fn a_flow_description_is_always_derived_from_its_graph_shape() {
    // The shared catalog type deliberately carries no free-text description.
    // Do not infer a purpose from a user-provided name: that would make the
    // prompt sound authoritative while merely guessing.
    let entry = entry_for(flow("Send invoices to accounting", true, vec![]));
    assert!(
        entry.description.starts_with("Saved Flows automation"),
        "fallback must describe the shape: {}",
        entry.description
    );
    // And it must not guess a purpose out of the name.
    assert!(!entry.description.contains("invoice"));
}

// ── Against a real store ──────────────────────────────────────────────────
//
// The mapping tests above never touch `flows.db`. These do, because the
// interesting failure is not the mapping — it is `flow_entries` reading the
// wrong database, or silently swallowing a real one. A unit test over
// `entry_for` would pass in both cases.

use tempfile::TempDir;

fn store_config(tmp: &TempDir) -> crate::config::Config {
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

#[test]
fn an_empty_store_contributes_nothing_to_the_catalogue() {
    // Also the common case: most workspaces have no flows, and the catalogue
    // must not grow a `[flow]` header or an empty group for them.
    let tmp = TempDir::new().unwrap();
    assert!(flow_entries(&store_config(&tmp)).is_empty());
}

#[test]
fn a_saved_flow_reaches_the_catalogue_with_its_real_id() {
    let tmp = TempDir::new().unwrap();
    let config = store_config(&tmp);

    let graph = WorkflowGraph {
        nodes: vec![
            node(
                "t",
                NodeKind::Trigger,
                serde_json::json!({ "trigger_kind": "manual" }),
            ),
            node("a", NodeKind::Agent, serde_json::json!({})),
        ],
        ..Default::default()
    };
    let saved =
        super::super::store::create_flow(&config, "Weekly Report".to_string(), graph, false, true)
            .expect("flow saves");

    let entries = flow_entries(&config);
    assert_eq!(entries.len(), 1, "the saved flow must appear: {entries:?}");
    let entry = &entries[0];
    assert_eq!(entry.name, "Weekly Report");
    // The store's generated id, not one this test made up — that is what
    // `run_workflow` will be handed.
    assert_eq!(entry.dir_name, saved.id);
    assert_eq!(entry.scope, WorkflowScope::Flow);
    assert!(entry.description.contains("manual trigger"));
    assert!(entry.description.contains("1 step)"));
}

#[test]
fn entries_are_sorted_by_name_so_the_prompt_prefix_is_stable() {
    // The catalogue is rendered into a system prompt that is frozen for a whole
    // session and cached by prefix. Insertion-order listing would reshuffle the
    // prefix whenever a flow was created, invalidating the cache for reasons
    // unrelated to the conversation.
    let tmp = TempDir::new().unwrap();
    let config = store_config(&tmp);
    for name in ["Zebra", "Alpha", "Mango"] {
        super::super::store::create_flow(
            &config,
            name.to_string(),
            WorkflowGraph::default(),
            false,
            true,
        )
        .expect("flow saves");
    }
    let names: Vec<String> = flow_entries(&config).into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["Alpha", "Mango", "Zebra"]);
}
