use super::*;

// compute_approval_manifest (save-time pre-authorization card)
pub(super) fn manifest_graph() -> WorkflowGraph {
    structurally_valid_graph(manifest_graph_json())
}

/// The same fixture in its wire form, for the RPCs that take a candidate
/// `graph` payload rather than a deserialized [`WorkflowGraph`].
pub(super) fn manifest_graph_json() -> Value {
    json!({
        "name": "manifest-fixture",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "h", "kind": "http_request", "name": "Call API",
              "config": { "url": "https://api.example.com/x", "method": "GET" } },
            { "id": "c", "kind": "code", "name": "Transform",
              "config": { "language": "javascript", "code": "return 1;" } },
            { "id": "w", "kind": "tool_call", "name": "Create order",
              "config": { "slug": "SHOPIFY_CREATE_ORDER" } },
            { "id": "r", "kind": "tool_call", "name": "Count products",
              "config": { "slug": "SHOPIFY_COUNT_PRODUCTS" } }
        ],
        "edges": [
            { "from_node": "t", "from_port": "main", "to_node": "h" },
            { "from_node": "h", "from_port": "main", "to_node": "c" },
            { "from_node": "c", "from_port": "main", "to_node": "w" },
            { "from_node": "w", "from_port": "main", "to_node": "r" }
        ]
    })
}

pub(super) fn entry_kinds_by_tool(entries: &[Value]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|entry| {
            (
                entry
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                entry
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
            )
        })
        .collect()
}
