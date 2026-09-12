use serde_json::json;
use tinyflows_catalog::import::n8n::map_n8n_workflow;

/// Imports a small n8n fixture that uses `$json` bindings end-to-end
/// through the exact same path `ops::flows_import` takes (map →
/// re-serialize → `validate_and_migrate_graph`), then runs the imported
/// graph through the domain's own binding-resolvability hard gate
/// (`ops::validate_binding_resolvability`, the same gate
/// `propose_workflow`/`revise_workflow`/`save_workflow` enforce before
/// accepting ANY graph — see `ops.rs`'s "Enforcing binding-resolvability
/// gate" section) and confirms it is accepted with zero errors, exactly
/// as `ops_tests.rs`'s `binding_to_agent_with_matching_schema_is_accepted`
/// exercises the gate for a hand-authored graph.
#[test]
fn imported_json_bindings_pass_binding_resolvability_and_resolve_the_real_item() {
    use crate::openhuman::flows::ops::{
        validate_and_migrate_graph, validate_binding_resolvability,
    };

    let wf = json!({
        "name": "json-binding-import",
        "nodes": [
            { "id": "t", "name": "Webhook", "type": "n8n-nodes-base.webhook" },
            { "id": "h", "name": "Notify", "type": "n8n-nodes-base.httpRequest",
              "parameters": {
                  "url": "={{ $json.callback_url }}",
                  "requestMethod": "POST"
              } }
        ],
        "connections": {
            "Webhook": { "main": [[{ "node": "Notify", "type": "main", "index": 0 }]] }
        }
    });

    let mapped = map_n8n_workflow(&wf).expect("map");
    // No warning for the `$json.callback_url` binding — it's the trivial
    // translatable case.
    assert!(
        !mapped
            .warnings
            .iter()
            .any(|w| w.contains("callback_url") || w.contains("not automatically translated")),
        "{:?}",
        mapped.warnings
    );

    let http_node = mapped.graph.node("h").expect("http node");
    assert_eq!(http_node.config["url"], json!("=.item.callback_url"));

    // Re-enter the same migrate + validate path `flows_import` uses.
    let value = serde_json::to_value(&mapped.graph).expect("serialize graph");
    let graph = validate_and_migrate_graph(value).expect("imported graph is structurally valid");

    // The domain's hard binding-resolvability gate accepts the imported
    // graph — the same gate a hand-authored graph must clear before
    // `propose_workflow`/`save_workflow` will accept it.
    assert!(
        validate_binding_resolvability(&graph).is_empty(),
        "{:?}",
        validate_binding_resolvability(&graph)
    );

    // The direct proof this whole importer exists to guarantee: the
    // translated binding actually resolves the real upstream item field
    // at runtime, evaluated against a scope shaped like the tinyflows
    // engine's real `expr_scope_for` — NOT null, which is exactly what
    // the pre-fix `=.callback_url` translation would have produced.
    let http_node = graph.node("h").expect("http node");
    let scope = json!({
        "item": { "callback_url": "https://example.com/hook" },
        "items": [{ "callback_url": "https://example.com/hook" }],
        "run": {},
        "nodes": {},
    });
    let resolved = tinyflows::expr::evaluate(&http_node.config["url"], &scope);
    assert_eq!(resolved, json!("https://example.com/hook"));
}
