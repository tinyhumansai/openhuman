//! Key-value and knowledge-graph RPC handlers for the unified memory store.

use serde::Deserialize;

use crate::openhuman::memory::api::provider::MemoryProvider;

use crate::rpc::RpcOutcome;

use super::guard::active_memory_guard;

/// Parameters for the `kv_set` RPC method.
#[derive(Debug, Deserialize)]
pub struct KvSetParams {
    /// The namespace for the key-value pair.
    #[serde(default)]
    pub namespace: Option<String>,
    /// The unique key.
    pub key: String,
    /// The value to store.
    pub value: serde_json::Value,
}

/// Parameters for `kv_get` and `kv_delete` RPC methods.
#[derive(Debug, Deserialize)]
pub struct KvGetDeleteParams {
    /// The namespace containing the key.
    #[serde(default)]
    pub namespace: Option<String>,
    /// The unique key.
    pub key: String,
}

/// Parameters for the `graph_upsert` RPC method.
#[derive(Debug, Deserialize)]
pub struct GraphUpsertParams {
    /// The namespace for the relation.
    #[serde(default)]
    pub namespace: Option<String>,
    /// The subject of the relation triple.
    pub subject: String,
    /// The predicate (relationship) of the triple.
    pub predicate: String,
    /// The object of the triple.
    pub object: String,
    /// Additional attributes for the relation.
    #[serde(default)]
    pub attrs: serde_json::Value,
}

/// Parameters for the `graph_query` RPC method.
#[derive(Debug, Deserialize)]
pub struct GraphQueryParams {
    /// The namespace to query.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Optional subject filter.
    #[serde(default)]
    pub subject: Option<String>,
    /// Optional predicate filter.
    #[serde(default)]
    pub predicate: Option<String>,
}

// ---------------------------------------------------------------------------
// KV handlers
// ---------------------------------------------------------------------------

/// Sets a key-value pair in the memory store.
///
/// Routed through [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard)
/// and the shared [`MemoryGraph`](crate::openhuman::memory::api::provider::MemoryGraph)
/// API, as are the other KV and graph handlers in this file.
pub async fn kv_set(params: KvSetParams) -> Result<RpcOutcome<bool>, String> {
    let guard = active_memory_guard().await?;
    let graph = guard
        .as_graph()
        .ok_or_else(|| "memory driver does not support the graph family".to_string())?;
    graph
        .kv_put(params.namespace.as_deref(), &params.key, params.value)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(true, "memory kv set"))
}

/// Retrieves a value by key from the memory store.
pub async fn kv_get(
    params: KvGetDeleteParams,
) -> Result<RpcOutcome<Option<serde_json::Value>>, String> {
    let guard = active_memory_guard().await?;
    let graph = guard
        .as_graph()
        .ok_or_else(|| "memory driver does not support the graph family".to_string())?;
    let value = graph
        .kv_get(params.namespace.as_deref(), &params.key)
        .await
        .map_err(|error| error.to_string())?
        .map(|record| record.value);
    Ok(RpcOutcome::single_log(value, "memory kv get"))
}

/// Deletes a key-value pair from the memory store.
pub async fn kv_delete(params: KvGetDeleteParams) -> Result<RpcOutcome<bool>, String> {
    let guard = active_memory_guard().await?;
    let graph = guard
        .as_graph()
        .ok_or_else(|| "memory driver does not support the graph family".to_string())?;
    let deleted = graph
        .kv_delete(params.namespace.as_deref(), &params.key)
        .await
        .map_err(|error| error.to_string())?;
    Ok(RpcOutcome::single_log(deleted, "memory kv delete"))
}

/// Lists all key-value entries in a namespace.
pub async fn kv_list_namespace(
    params: super::documents::NamespaceOnlyParams,
) -> Result<RpcOutcome<Vec<serde_json::Value>>, String> {
    let guard = active_memory_guard().await?;
    let graph = guard
        .as_graph()
        .ok_or_else(|| "memory driver does not support the graph family".to_string())?;
    let rows = graph
        .kv_list(Some(&params.namespace), None, usize::MAX)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|record| serde_json::to_value(record).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RpcOutcome::single_log(rows, "memory namespace kv listed"))
}

// ---------------------------------------------------------------------------
// Graph handlers
// ---------------------------------------------------------------------------

/// Upserts a relation triple in the knowledge graph.
pub async fn graph_upsert(params: GraphUpsertParams) -> Result<RpcOutcome<bool>, String> {
    let guard = active_memory_guard().await?;
    let graph = guard
        .as_graph()
        .ok_or_else(|| "memory driver does not support the graph family".to_string())?;
    graph
        .put_relation(crate::openhuman::memory::api::types::GraphRelationRecord {
            namespace: params.namespace,
            subject: params.subject,
            predicate: params.predicate,
            object: params.object,
            attrs: params.attrs,
            updated_at: 0.0,
            evidence_count: 1,
            order_index: None,
            document_ids: vec![],
            chunk_ids: vec![],
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(RpcOutcome::single_log(true, "memory graph upserted"))
}

/// Queries relations from the knowledge graph.
pub async fn graph_query(
    params: GraphQueryParams,
) -> Result<RpcOutcome<Vec<serde_json::Value>>, String> {
    let guard = active_memory_guard().await?;
    let graph = guard
        .as_graph()
        .ok_or_else(|| "memory driver does not support the graph family".to_string())?;
    let rows = graph
        .relations(
            params.namespace.as_deref(),
            params.subject.as_deref(),
            params.predicate.as_deref(),
            usize::MAX,
        )
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|record| serde_json::to_value(record).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RpcOutcome::single_log(rows, "memory graph queried"))
}

#[cfg(test)]
#[path = "kv_graph_tests.rs"]
mod tests;
