use super::*;
use tinyflows_bus::node_kinds::NODE_KINDS;

async fn contract(kind: &str) -> tinyflows_bus::node_kinds::NodeKindContract {
    node_kind_contract(&crate::config::Config::default(), kind)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn overlay_preserves_every_kind_from_the_catalog_bus() {
    let contracts = all_node_kind_contracts(&crate::config::Config::default())
        .await
        .unwrap();
    assert_eq!(contracts.len(), NODE_KINDS.len());
    for kind in NODE_KINDS {
        assert!(
            contracts.iter().any(|contract| contract.kind == kind),
            "missing {kind}"
        );
    }
    assert!(
        node_kind_contract(&crate::config::Config::default(), "not_a_kind")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn memory_overlay_adds_flow_memory_coherence_facts_and_redirects_dedup_to_its_own_node() {
    let c = contract("memory").await;
    let notes = c.notes.join("\n");
    assert!(notes.contains("flow_memory_recall"), "{notes}");
    assert!(notes.contains("flow_memory_remember"), "{notes}");
    assert!(notes.contains("SAME per-flow memory namespace"), "{notes}");
    assert!(!notes.contains("Canonical dedupe pattern"), "{notes}");
    assert!(!notes.contains("item.json.found"), "{notes}");
    assert!(
        !notes.contains("deferred to a dedicated primitive"),
        "{notes}"
    );
    assert!(notes.contains("use a dedup node instead"), "{notes}");
}

#[tokio::test]
async fn dedup_overlay_teaches_run_level_commit_semantics_and_placement() {
    let c = contract("dedup").await;
    let notes = c.notes.join("\n");
    assert!(notes.contains("FlowRunFinished"), "{notes}");
    assert!(notes.contains("completed_with_warnings"), "{notes}");
    assert!(notes.contains("failed/cancelled/interrupted"), "{notes}");
    assert!(notes.contains("unknown"), "{notes}");
    assert!(notes.contains("split_out → dedup"), "{notes}");
}

#[tokio::test]
async fn tool_call_overlay_adds_host_composio_facts() {
    let c = contract("tool_call").await;
    let notes = c.notes.join("\n");
    assert!(notes.contains("Composio"), "{notes}");
    assert!(notes.contains("oh:"), "{notes}");
    assert!(notes.contains("data"), "{notes}");
    assert!(notes.contains("get_tool_contract"), "{notes}");
}

#[tokio::test]
async fn agent_overlay_adds_input_context_guidance() {
    let c = contract("agent").await;
    assert!(c.notes.iter().any(|n| n.contains("input_context")));
}

#[tokio::test]
async fn trigger_overlay_names_the_host_dispatch_set() {
    let c = contract("trigger").await;
    assert!(c.notes.iter().any(|n| n.contains("app_event")));
}

#[tokio::test]
async fn merge_keeps_the_module_contract_without_a_host_overlay() {
    let c = contract("merge").await;
    assert!(!c.notes.iter().any(|note| note.contains("THIS host")));
    assert!(!c.notes.iter().any(|note| note.contains("Composio")));
}
