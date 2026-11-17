//! Read-only traversal helpers for TinyFlows' normalized workflow JSON.

use serde_json::Value;

pub(crate) struct GraphNode<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) config: &'a Value,
}

/// Rejects malformed node identity fields before a host callback evaluates a
/// gate. TinyFlows has already normalized the graph; this preserves the bus
/// boundary's required JSON shape without parsing an engine-owned AST here.
pub(crate) fn validate_host_gate_input_shape(graph: &Value) -> Result<(), String> {
    let nodes = graph
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "workflow graph must contain a node array".to_string())?;
    for (index, node) in nodes.iter().enumerate() {
        if node.get("id").and_then(Value::as_str).is_none() {
            return Err(format!(
                "workflow graph node {index} must have a string `id`"
            ));
        }
        if node.get("kind").and_then(Value::as_str).is_none() {
            return Err(format!(
                "workflow graph node {index} must have a string `kind`"
            ));
        }
    }
    Ok(())
}

/// Iterates structurally valid nodes from TinyFlows' normalized graph JSON.
pub(crate) fn nodes(graph: &Value) -> impl Iterator<Item = GraphNode<'_>> {
    graph
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|node| {
            Some(GraphNode {
                id: node.get("id")?.as_str()?,
                kind: node.get("kind")?.as_str()?,
                config: node.get("config").unwrap_or(&Value::Null),
            })
        })
}
