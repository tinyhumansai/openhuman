//! Unit tests for memory RPC schema registration and parameter parsing,
//! validating that every advertised function name has a registered controller.

use super::*;
use serde_json::json;

const ALL_FUNCTIONS: &[&str] = &[
    "init",
    "list_documents",
    "list_namespaces",
    "namespace_summaries",
    "delete_document",
    "query_namespace",
    "recall_context",
    "recall_memories",
    "list_files",
    "read_file",
    "write_file",
    "namespace_list",
    "doc_put",
    "doc_ingest",
    "doc_list",
    "doc_delete",
    "context_query",
    "context_recall",
    "kv_set",
    "kv_get",
    "kv_delete",
    "kv_list_namespace",
    "graph_upsert",
    "graph_query",
    "clear_namespace",
    "sync_channel",
    "sync_all",
    "learn_all",
    "ingestion_status",
    "scheduler_override",
    // The bound memory driver (kernel.md §6 item 6, plan-memory.md §5)
    "provider_status",
    // Tool-scoped memory (#1400)
    "tool_rule_put",
    "tool_rule_get",
    "tool_rule_list",
    "tool_rule_delete",
    "tool_rules_for_prompt",
    "tool_rules_json",
];

/// The exact ordered `memory.*` registration sequence. Order matters: it is the
/// order `src/core/all.rs` pushes controllers in, which is the order `/schema`
/// and the CLI catalog advertise them in.
///
/// Unlike [`ALL_FUNCTIONS`] — an unordered membership list — this is ordered
/// and must not be edited to accommodate a refactor. If a refactor makes this
/// fail, the refactor is a behaviour change.
///
/// **Edited once, deliberately, by M5.2.** The `documents` family was split
/// into three capability partitions (core+recall / documents / ingest) so the
/// gated ones can be registered independently of the MANDATORY ones — tagging
/// the whole file `Capability::Documents` would have made `recall_memories`
/// vanish under a driver that merely lacks the document tier. The only
/// consequence visible here is that `doc_ingest` moved from between `doc_put`
/// and `doc_list` to after `doc_delete`, and the three `context_*`/`clear_*`
/// functions moved ahead of the `doc_*` block. Membership is unchanged, every
/// method name is unchanged, and registration order within a namespace carries
/// no wire semantics (dispatch is by method name; `/schema` is a list). This is
/// the M5.2 change, not licence to re-edit the list for the next refactor.
const REGISTRATION_ORDER: &[&str] = &[
    // documents — core + recall partition (mandatory, never capability-gated)
    "init",
    "list_documents",
    "list_namespaces",
    "namespace_summaries",
    "delete_document",
    "query_namespace",
    "recall_context",
    "recall_memories",
    "namespace_list",
    "context_query",
    "context_recall",
    "clear_namespace",
    // documents — namespace-document tier (Capability::Documents)
    "doc_put",
    "doc_list",
    "doc_delete",
    // documents — driver-owned ingestion (Capability::Ingest)
    "doc_ingest",
    // files
    "list_files",
    "read_file",
    "write_file",
    // kv_graph
    "kv_set",
    "kv_get",
    "kv_delete",
    "kv_list_namespace",
    "graph_upsert",
    "graph_query",
    // sync
    "sync_channel",
    "sync_all",
    "ingestion_status",
    "scheduler_override",
    // learn
    "learn_all",
    // provider
    "provider_status",
    // tool_memory
    "tool_rule_put",
    "tool_rule_get",
    "tool_rule_list",
    "tool_rule_delete",
    "tool_rules_for_prompt",
    "tool_rules_json",
];

fn functions_of(controllers: &[RegisteredController]) -> Vec<&'static str> {
    controllers.iter().map(|c| c.schema.function).collect()
}

#[test]
fn registered_controller_order_is_pinned_to_the_capability_partition_snapshot() {
    assert_eq!(
        ALL_FUNCTIONS.len(),
        REGISTRATION_ORDER.len(),
        "the membership list and the ordered list have drifted apart"
    );
    assert_eq!(
        functions_of(&all_registered_controllers()),
        REGISTRATION_ORDER,
        "memory controller registration order changed — this is a behaviour change, not a refactor"
    );
}

#[test]
fn controller_schema_order_is_pinned_to_pre_split_snapshot() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names, REGISTRATION_ORDER);
}

#[test]
fn aggregator_is_exactly_the_nine_parts_concatenated_in_order() {
    let mut expected = Vec::new();
    expected.extend(functions_of(&all_core_recall_registered_controllers()));
    expected.extend(functions_of(&all_documents_registered_controllers()));
    expected.extend(functions_of(&all_ingest_registered_controllers()));
    expected.extend(functions_of(&all_files_registered_controllers()));
    expected.extend(functions_of(&all_kv_graph_registered_controllers()));
    expected.extend(functions_of(&all_sync_registered_controllers()));
    expected.extend(functions_of(&all_learn_registered_controllers()));
    expected.extend(functions_of(&all_provider_registered_controllers()));
    expected.extend(functions_of(&all_tool_memory_registered_controllers()));

    assert_eq!(functions_of(&all_registered_controllers()), expected);
    assert_eq!(expected, REGISTRATION_ORDER);
}

#[test]
fn schema_aggregator_is_exactly_the_nine_parts_concatenated_in_order() {
    let mut expected: Vec<&'static str> = Vec::new();
    for family in [
        all_core_recall_controller_schemas(),
        all_documents_controller_schemas(),
        all_ingest_controller_schemas(),
        all_files_controller_schemas(),
        all_kv_graph_controller_schemas(),
        all_sync_controller_schemas(),
        all_learn_controller_schemas(),
        all_provider_controller_schemas(),
        all_tool_memory_controller_schemas(),
    ] {
        expected.extend(family.into_iter().map(|s| s.function));
    }
    let actual: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn each_family_pairs_its_schemas_with_its_controllers() {
    let families: [(&str, Vec<ControllerSchema>, Vec<RegisteredController>); 9] = [
        (
            "core_recall",
            all_core_recall_controller_schemas(),
            all_core_recall_registered_controllers(),
        ),
        (
            "documents",
            all_documents_controller_schemas(),
            all_documents_registered_controllers(),
        ),
        (
            "ingest",
            all_ingest_controller_schemas(),
            all_ingest_registered_controllers(),
        ),
        (
            "files",
            all_files_controller_schemas(),
            all_files_registered_controllers(),
        ),
        (
            "kv_graph",
            all_kv_graph_controller_schemas(),
            all_kv_graph_registered_controllers(),
        ),
        (
            "sync",
            all_sync_controller_schemas(),
            all_sync_registered_controllers(),
        ),
        (
            "learn",
            all_learn_controller_schemas(),
            all_learn_registered_controllers(),
        ),
        (
            "provider",
            all_provider_controller_schemas(),
            all_provider_registered_controllers(),
        ),
        (
            "tool_memory",
            all_tool_memory_controller_schemas(),
            all_tool_memory_registered_controllers(),
        ),
    ];

    let mut total = 0;
    for (name, schemas, controllers) in families {
        assert!(!schemas.is_empty(), "family {name} advertises no schemas");
        let schema_fns: Vec<_> = schemas.iter().map(|s| s.function).collect();
        assert_eq!(
            schema_fns,
            functions_of(&controllers),
            "family {name} schema order diverges from its handler order"
        );
        total += controllers.len();
    }
    assert_eq!(
        total,
        REGISTRATION_ORDER.len(),
        "the nine parts must cover the whole memory surface — no function may be orphaned"
    );
}

#[test]
fn all_controller_schemas_has_entry_per_supported_function() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names.len(), ALL_FUNCTIONS.len());
    for expected in ALL_FUNCTIONS {
        assert!(names.contains(expected), "missing schema for {expected}");
    }
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), ALL_FUNCTIONS.len());
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    for expected in ALL_FUNCTIONS {
        assert!(names.contains(expected), "missing handler for {expected}");
    }
}

#[test]
fn every_schema_uses_memory_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(
            s.namespace, "memory",
            "schema {} must use the memory namespace",
            s.function
        );
    }
}

#[test]
fn every_schema_has_a_non_empty_description() {
    for s in all_controller_schemas() {
        assert!(
            !s.description.is_empty(),
            "schema {} has empty description",
            s.function
        );
    }
}

#[test]
fn schemas_unknown_function_returns_unknown_placeholder() {
    let s = schemas("not-a-real-function");
    assert_eq!(s.namespace, "memory");
    assert_eq!(s.function, "unknown");
}

// ── parse_params helper ──────────────────────────────────────

#[test]
fn parse_params_deserializes_simple_struct() {
    #[derive(serde::Deserialize, Debug)]
    struct Simple {
        name: String,
        count: u32,
    }
    let mut m = Map::new();
    m.insert("name".into(), json!("hi"));
    m.insert("count".into(), json!(7));
    let out: Simple = parse_params(m).unwrap();
    assert_eq!(out.name, "hi");
    assert_eq!(out.count, 7);
}

#[test]
fn parse_params_surfaces_deserialization_errors_with_context() {
    #[derive(serde::Deserialize, Debug)]
    struct Strict {
        #[allow(dead_code)]
        count: u32,
    }
    let mut m = Map::new();
    m.insert("count".into(), json!("not-a-number"));
    let err = parse_params::<Strict>(m).unwrap_err();
    assert!(err.contains("invalid params"));
}

// ── sync / learn schema shape tests ─────────────────────────────────────

#[test]
fn sync_channel_schema_requires_channel_id() {
    let s = schemas("sync_channel");
    assert_eq!(s.namespace, "memory");
    assert_eq!(s.function, "sync_channel");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(
        required.contains(&"channel_id"),
        "channel_id must be required"
    );
}

#[test]
fn sync_all_schema_has_no_inputs() {
    let s = schemas("sync_all");
    assert_eq!(s.function, "sync_all");
    assert!(s.inputs.is_empty(), "sync_all takes no inputs");
}

#[test]
fn learn_all_schema_namespaces_is_optional() {
    let s = schemas("learn_all");
    assert_eq!(s.function, "learn_all");
    assert_eq!(s.inputs.len(), 1);
    let ns_field = &s.inputs[0];
    assert_eq!(ns_field.name, "namespaces");
    assert!(!ns_field.required, "namespaces must be optional");
}
