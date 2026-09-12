use super::*;

#[test]
fn overlay_preserves_every_kind() {
    // Counted from NODE_KINDS rather than a literal: the overlay must keep
    // pace with the engine's catalog, and pinning a number here only ever
    // reported "tinyflows added a kind", which is not this test's job.
    assert_eq!(all_node_kind_contracts().len(), NODE_KINDS.len());
    for kind in NODE_KINDS {
        assert!(node_kind_contract(kind).is_some(), "missing {kind}");
    }
    assert!(node_kind_contract("not_a_kind").is_none());
}

#[test]
fn memory_overlay_adds_flow_memory_coherence_facts_and_redirects_dedup_to_its_own_node() {
    let c = node_kind_contract("memory").unwrap();
    let notes = c.notes.join("\n");
    assert!(notes.contains("flow_memory_recall"), "{notes}");
    assert!(notes.contains("flow_memory_remember"), "{notes}");
    assert!(notes.contains("SAME per-flow memory namespace"), "{notes}");
    // The recall→condition dedupe recipe stays gone (P1 review fix):
    // semantic recall cannot express exact "have I seen this key"
    // membership, so the overlay must not teach that pattern.
    assert!(!notes.contains("Canonical dedupe pattern"), "{notes}");
    assert!(!notes.contains("item.json.found"), "{notes}");
    // The "deferred to a dedicated primitive" note is gone now that the
    // dedup node exists — the memory overlay redirects to it instead.
    assert!(
        !notes.contains("deferred to a dedicated primitive"),
        "{notes}"
    );
    assert!(notes.contains("use a dedup node instead"), "{notes}");
}

#[test]
fn dedup_overlay_teaches_run_level_commit_semantics_and_placement() {
    let c = node_kind_contract("dedup").unwrap();
    let notes = c.notes.join("\n");
    assert!(notes.contains("FlowRunFinished"), "{notes}");
    assert!(notes.contains("completed_with_warnings"), "{notes}");
    assert!(notes.contains("failed/cancelled/interrupted"), "{notes}");
    // CodeRabbit (PR #5265): the release path is really "every status
    // other than the two success strings" — `unknown` and any future
    // status must be documented alongside the known failure statuses.
    assert!(notes.contains("unknown"), "{notes}");
    assert!(notes.contains("split_out → dedup"), "{notes}");
}

#[test]
fn tool_call_overlay_adds_host_composio_facts() {
    let c = node_kind_contract("tool_call").unwrap();
    let notes = c.notes.join("\n");
    // Host facts that must NOT live in the portable crate.
    assert!(notes.contains("Composio"), "{notes}");
    assert!(notes.contains("oh:"), "{notes}");
    assert!(notes.contains("data"), "{notes}");
    assert!(notes.contains("get_tool_contract"), "{notes}");
}

#[test]
fn agent_overlay_adds_input_context_guidance() {
    let c = node_kind_contract("agent").unwrap();
    assert!(c.notes.iter().any(|n| n.contains("input_context")));
}

#[test]
fn trigger_overlay_names_the_host_dispatch_set() {
    let c = node_kind_contract("trigger").unwrap();
    assert!(c.notes.iter().any(|n| n.contains("app_event")));
}

#[test]
fn merge_has_no_overlay_and_stays_portable() {
    // A kind with no host facts is byte-identical to the portable contract.
    assert_eq!(
        node_kind_contract("merge").unwrap(),
        tinyflows::catalog::contract_for("merge").unwrap()
    );
}

#[test]
fn rendered_line_covers_every_kind_and_required_field() {
    let line = render_node_kinds_line();
    for c in all_node_kind_contracts() {
        assert!(
            line.contains(&c.kind),
            "rendered line missing kind {}",
            c.kind
        );
        for f in c.config_fields.iter().filter(|f| f.required) {
            assert!(
                line.contains(&format!("config.{}", f.name)),
                "rendered line missing required field config.{} for {}",
                f.name,
                c.kind
            );
        }
    }
}
