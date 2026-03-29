use crate::openhuman::memory::{MemoryClient, NamespaceDocumentInput};
use crate::openhuman::rpc::RpcOutcome;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct PutDocParams {
    pub namespace: String,
    pub key: String,
    pub title: String,
    pub content: String,
    #[serde(default = "default_source_type")]
    pub source_type: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub document_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NamespaceOnlyParams {
    pub namespace: String,
}

#[derive(Debug, Deserialize)]
pub struct OptionalNamespaceParams {
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteDocParams {
    pub namespace: String,
    pub document_id: String,
}

#[derive(Debug, Deserialize)]
pub struct QueryNamespaceParams {
    pub namespace: String,
    pub query: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct RecallNamespaceParams {
    pub namespace: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct KvSetParams {
    #[serde(default)]
    pub namespace: Option<String>,
    pub key: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct KvGetDeleteParams {
    #[serde(default)]
    pub namespace: Option<String>,
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub struct GraphUpsertParams {
    #[serde(default)]
    pub namespace: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(default)]
    pub attrs: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct GraphQueryParams {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub predicate: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PutDocResult {
    pub document_id: String,
}

fn default_source_type() -> String {
    "doc".to_string()
}

fn default_priority() -> String {
    "medium".to_string()
}

fn default_category() -> String {
    "core".to_string()
}

pub async fn namespace_list() -> Result<RpcOutcome<Vec<String>>, String> {
    let client = MemoryClient::new_local()?;
    let namespaces = client.list_namespaces().await?;
    Ok(RpcOutcome::single_log(
        namespaces,
        "memory namespaces listed",
    ))
}

pub async fn doc_put(params: PutDocParams) -> Result<RpcOutcome<PutDocResult>, String> {
    let client = MemoryClient::new_local()?;
    let document_id = client
        .put_doc(NamespaceDocumentInput {
            namespace: params.namespace,
            key: params.key,
            title: params.title,
            content: params.content,
            source_type: params.source_type,
            priority: params.priority,
            tags: params.tags,
            metadata: params.metadata,
            category: params.category,
            session_id: params.session_id,
            document_id: params.document_id,
        })
        .await?;
    Ok(RpcOutcome::single_log(
        PutDocResult { document_id },
        "memory document upserted",
    ))
}

pub async fn doc_list(
    params: Option<NamespaceOnlyParams>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let client = MemoryClient::new_local()?;
    let docs = client
        .list_documents(params.as_ref().map(|v| v.namespace.as_str()))
        .await?;
    Ok(RpcOutcome::single_log(docs, "memory documents listed"))
}

pub async fn doc_delete(params: DeleteDocParams) -> Result<RpcOutcome<serde_json::Value>, String> {
    let client = MemoryClient::new_local()?;
    let result = client
        .delete_document(&params.namespace, &params.document_id)
        .await?;
    Ok(RpcOutcome::single_log(result, "memory document deleted"))
}

pub async fn context_query(params: QueryNamespaceParams) -> Result<RpcOutcome<String>, String> {
    let client = MemoryClient::new_local()?;
    let result = client
        .query_namespace(&params.namespace, &params.query, params.limit.unwrap_or(10))
        .await?;
    Ok(RpcOutcome::single_log(result, "memory context queried"))
}

pub async fn context_recall(
    params: RecallNamespaceParams,
) -> Result<RpcOutcome<Option<String>>, String> {
    let client = MemoryClient::new_local()?;
    let result = client
        .recall_namespace(&params.namespace, params.limit.unwrap_or(10))
        .await?;
    Ok(RpcOutcome::single_log(result, "memory context recalled"))
}

pub async fn kv_set(params: KvSetParams) -> Result<RpcOutcome<bool>, String> {
    let client = MemoryClient::new_local()?;
    client
        .kv_set(params.namespace.as_deref(), &params.key, &params.value)
        .await?;
    Ok(RpcOutcome::single_log(true, "memory kv set"))
}

pub async fn kv_get(
    params: KvGetDeleteParams,
) -> Result<RpcOutcome<Option<serde_json::Value>>, String> {
    let client = MemoryClient::new_local()?;
    let value = client
        .kv_get(params.namespace.as_deref(), &params.key)
        .await?;
    Ok(RpcOutcome::single_log(value, "memory kv get"))
}

pub async fn kv_delete(params: KvGetDeleteParams) -> Result<RpcOutcome<bool>, String> {
    let client = MemoryClient::new_local()?;
    let deleted = client
        .kv_delete(params.namespace.as_deref(), &params.key)
        .await?;
    Ok(RpcOutcome::single_log(deleted, "memory kv delete"))
}

pub async fn kv_list_namespace(
    params: NamespaceOnlyParams,
) -> Result<RpcOutcome<Vec<serde_json::Value>>, String> {
    let client = MemoryClient::new_local()?;
    let rows = client.kv_list_namespace(&params.namespace).await?;
    Ok(RpcOutcome::single_log(rows, "memory namespace kv listed"))
}

pub async fn graph_upsert(params: GraphUpsertParams) -> Result<RpcOutcome<bool>, String> {
    let client = MemoryClient::new_local()?;
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

pub async fn graph_query(
    params: GraphQueryParams,
) -> Result<RpcOutcome<Vec<serde_json::Value>>, String> {
    let client = MemoryClient::new_local()?;
    let rows = client
        .graph_query(
            params.namespace.as_deref(),
            params.subject.as_deref(),
            params.predicate.as_deref(),
        )
        .await?;
    Ok(RpcOutcome::single_log(rows, "memory graph queried"))
}
