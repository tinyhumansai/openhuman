//! Key-value and knowledge-graph RPC handlers for the unified memory store.

use serde::Deserialize;

use crate::rpc::RpcOutcome;

use super::helpers::active_memory_client;

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
pub async fn kv_set(params: KvSetParams) -> Result<RpcOutcome<bool>, String> {
    let client = active_memory_client().await?;
    client
        .kv_set(params.namespace.as_deref(), &params.key, &params.value)
        .await?;
    Ok(RpcOutcome::single_log(true, "memory kv set"))
}

/// Retrieves a value by key from the memory store.
pub async fn kv_get(
    params: KvGetDeleteParams,
) -> Result<RpcOutcome<Option<serde_json::Value>>, String> {
    let client = active_memory_client().await?;
    let value = client
        .kv_get(params.namespace.as_deref(), &params.key)
        .await?;
    Ok(RpcOutcome::single_log(value, "memory kv get"))
}

/// Deletes a key-value pair from the memory store.
pub async fn kv_delete(params: KvGetDeleteParams) -> Result<RpcOutcome<bool>, String> {
    let client = active_memory_client().await?;
    let deleted = client
        .kv_delete(params.namespace.as_deref(), &params.key)
        .await?;
    Ok(RpcOutcome::single_log(deleted, "memory kv delete"))
}

/// Lists all key-value entries in a namespace.
pub async fn kv_list_namespace(
    params: super::documents::NamespaceOnlyParams,
) -> Result<RpcOutcome<Vec<serde_json::Value>>, String> {
    let client = active_memory_client().await?;
    let rows = client.kv_list_namespace(&params.namespace).await?;
    Ok(RpcOutcome::single_log(rows, "memory namespace kv listed"))
}

// ---------------------------------------------------------------------------
// Graph handlers
// ---------------------------------------------------------------------------

/// Upserts a relation triple in the knowledge graph.
pub async fn graph_upsert(params: GraphUpsertParams) -> Result<RpcOutcome<bool>, String> {
    let client = active_memory_client().await?;
    client
        .graph_upsert(
            params.namespace.as_deref(),
            &params.subject,
            &params.predicate,
            &params.object,
            &params.attrs,
        )
        .await?;
    Ok(RpcOutcome::single_log(true, "memory graph upserted"))
}

/// Queries relations from the knowledge graph.
pub async fn graph_query(
    params: GraphQueryParams,
) -> Result<RpcOutcome<Vec<serde_json::Value>>, String> {
    let client = active_memory_client().await?;
    let rows = client
        .graph_query(
            params.namespace.as_deref(),
            params.subject.as_deref(),
            params.predicate.as_deref(),
        )
        .await?;
    Ok(RpcOutcome::single_log(rows, "memory graph queried"))
}
