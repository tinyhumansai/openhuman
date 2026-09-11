//! Sibling tests for the tree-mode graph export, covering the shaping that
//! stayed host-side when `summary_forest` moved onto the memory module (#5560).
//!
//! These assertions used to live in `tests/memory_sync_pipeline_e2e.rs`, driven
//! through `graph_export_rpc` against an in-process engine. That route is gone:
//! the forest is served by the module now, so the same test would need a loaded
//! module to say anything at all about node shape — and it would then be
//! asserting the driver's storage as much as the host's rendering. What is
//! actually worth pinning here is the second half, and none of it needs a store:
//!
//! 1. **A synthetic source root per scope, parented to nothing.** The forest has
//!    no such node; the export invents one so the UI has something to hang a
//!    tree off. Its id is derived (`source:<scope>`), so a scope that collides
//!    with a real summary id would silently merge two trees.
//! 2. **An orphan summary is re-parented onto its source root, and only an
//!    orphan.** "Orphan" means the parent it names is not itself in the forest —
//!    which is the normal state of an L1 seal, not an error. A summary whose
//!    parent *is* present must keep it, or a truncated forest would quietly
//!    reshape into a flat one.
//! 3. **Document leaves come from an L1 summary's child ids, but only when
//!    those children are raw items.** A summary that sealed over other
//!    summaries already has its children in the forest as real nodes; emitting
//!    document leaves for them too would double every interior node.
//! 4. **The node budget cuts the document tail, never the summaries.** The
//!    summaries are the skeleton — a budget that dropped them would leave
//!    document leaves parented to nodes that are not in the response.

use chrono::{TimeZone, Utc};

use super::shape_summary_nodes;
use crate::openhuman::memory::api::tree::TreeSummary;

const BUDGET: usize = 10_000;

fn summary(
    id: &str,
    scope: &str,
    level: u32,
    parent: Option<&str>,
    children: &[&str],
) -> TreeSummary {
    TreeSummary {
        id: id.to_string(),
        tree_id: format!("tree-{scope}"),
        tree_kind: "source".to_string(),
        tree_scope: scope.to_string(),
        level,
        parent_id: parent.map(str::to_string),
        child_ids: children.iter().map(|c| (*c).to_string()).collect(),
        time_range_start: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        time_range_end: Utc.timestamp_opt(1_700_000_900, 0).unwrap(),
    }
}

#[test]
fn each_scope_gets_one_synthetic_source_root_with_no_parent() {
    let nodes = shape_summary_nodes(
        vec![
            summary("summary:a", "github:acme/widgets", 1, None, &["commit:aaa"]),
            summary("summary:b", "github:acme/gadgets", 1, None, &["commit:bbb"]),
            // A second summary in an already-seen scope must not add a root.
            summary("summary:c", "github:acme/widgets", 1, None, &["commit:ccc"]),
        ],
        BUDGET,
    );

    let roots: Vec<_> = nodes.iter().filter(|n| n.kind == "source").collect();
    assert_eq!(roots.len(), 2, "one root per distinct scope, got {roots:?}");
    for root in &roots {
        assert_eq!(root.parent_id, None, "a source root has no parent");
    }
    assert!(roots.iter().any(|n| n.id == "source:github:acme/widgets"));
    assert!(roots.iter().any(|n| n.id == "source:github:acme/gadgets"));
}

#[test]
fn an_orphan_summary_is_reparented_onto_its_source_root() {
    // `parent_id` names a summary that is not in the forest — the ordinary
    // shape of an L1 seal that has not been rolled up yet.
    let nodes = shape_summary_nodes(
        vec![summary(
            "summary:l1",
            "github:acme/widgets",
            1,
            Some("summary:absent"),
            &["commit:aaa"],
        )],
        BUDGET,
    );

    let l1 = nodes
        .iter()
        .find(|n| n.id == "summary:l1")
        .expect("the summary is in the graph");
    assert_eq!(
        l1.parent_id.as_deref(),
        Some("source:github:acme/widgets"),
        "an unresolvable parent means orphan, so it hangs off the source root"
    );
}

#[test]
fn a_summary_whose_parent_is_present_keeps_it() {
    let nodes = shape_summary_nodes(
        vec![
            summary(
                "summary:l2",
                "github:acme/widgets",
                2,
                None,
                &["summary:l1"],
            ),
            summary(
                "summary:l1",
                "github:acme/widgets",
                1,
                Some("summary:l2"),
                &["commit:aaa"],
            ),
        ],
        BUDGET,
    );

    let l1 = nodes
        .iter()
        .find(|n| n.id == "summary:l1")
        .expect("the L1 summary is in the graph");
    assert_eq!(
        l1.parent_id.as_deref(),
        Some("summary:l2"),
        "a real parent in the forest must survive the re-parenting pass"
    );
}

#[test]
fn document_leaves_are_emitted_for_an_l1_summary_over_raw_items() {
    let nodes = shape_summary_nodes(
        vec![summary(
            "summary:l1",
            "github:acme/widgets",
            1,
            None,
            &["commit:aaa111", "issue:42", "pr:7"],
        )],
        BUDGET,
    );

    let docs: Vec<_> = nodes
        .iter()
        .filter(|n| n.kind == "chunk" && n.parent_id.as_deref() == Some("summary:l1"))
        .collect();
    assert_eq!(
        docs.len(),
        3,
        "one document leaf per raw child id, got {docs:?}"
    );
    assert!(
        docs.iter().any(|n| n.id.contains("commit:aaa111")),
        "the child id is carried into the leaf's own id"
    );
}

#[test]
fn a_summary_that_sealed_over_summaries_emits_no_document_leaves() {
    // Its children are already in the forest as real summary nodes; emitting
    // document leaves for them as well would render every interior node twice.
    let nodes = shape_summary_nodes(
        vec![
            summary(
                "summary:l1",
                "github:acme/widgets",
                1,
                None,
                &["summary:child-a", "summary:child-b"],
            ),
            summary("summary:child-a", "github:acme/widgets", 1, None, &[]),
        ],
        BUDGET,
    );

    let docs: Vec<_> = nodes
        .iter()
        .filter(|n| n.kind == "chunk" && n.parent_id.as_deref() == Some("summary:l1"))
        .collect();
    assert!(docs.is_empty(), "expected no document leaves, got {docs:?}");
}

#[test]
fn a_non_l1_summary_emits_no_document_leaves() {
    let nodes = shape_summary_nodes(
        vec![summary(
            "summary:l2",
            "github:acme/widgets",
            2,
            None,
            &["commit:aaa"],
        )],
        BUDGET,
    );

    assert!(
        !nodes.iter().any(|n| n.kind == "chunk"),
        "document leaves hang off L1 only, got {nodes:?}"
    );
}

#[test]
fn the_budget_cuts_the_document_tail_and_keeps_every_summary() {
    // One scope (1 root) + one summary = 2 nodes before any document leaf, so
    // a budget of 3 leaves room for exactly one of the three children.
    let nodes = shape_summary_nodes(
        vec![summary(
            "summary:l1",
            "github:acme/widgets",
            1,
            None,
            &["commit:aaa", "commit:bbb", "commit:ccc"],
        )],
        3,
    );

    assert_eq!(
        nodes.iter().filter(|n| n.kind == "source").count(),
        1,
        "the source root survives the budget"
    );
    assert_eq!(
        nodes.iter().filter(|n| n.kind == "summary").count(),
        1,
        "the summary skeleton survives the budget"
    );
    assert_eq!(
        nodes.iter().filter(|n| n.kind == "chunk").count(),
        1,
        "only the document tail is cut, got {nodes:?}"
    );
}

#[test]
fn an_empty_forest_shapes_to_no_nodes_at_all() {
    // Not even a source root: roots are derived from the scopes present, so a
    // store that has sealed nothing renders as empty rather than as one bare
    // root per scope it has never seen.
    assert!(shape_summary_nodes(Vec::new(), BUDGET).is_empty());
}
