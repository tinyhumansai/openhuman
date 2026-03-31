use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::rpc::RpcOutcome;

fn parse_params<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

fn rpc_json<T: Serialize>(outcome: RpcOutcome<T>) -> Result<serde_json::Value, String> {
    outcome.into_cli_compatible_json()
}

#[derive(Debug, Deserialize)]
struct MemoryDocListParams {
    namespace: Option<String>,
}

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    match method {
        "memory.init" => Some(
            async move {
                let payload: crate::openhuman::memory::MemoryInitRequest = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::memory_init(payload).await?)
            }
            .await,
        ),

        "memory.list_documents" => Some(
            async move {
                let payload: crate::openhuman::memory::ListDocumentsRequest = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::memory_list_documents(payload).await?)
            }
            .await,
        ),

        "memory.list_namespaces" => Some(
            async move {
                let payload: crate::openhuman::memory::EmptyRequest = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::memory_list_namespaces(payload).await?)
            }
            .await,
        ),

        "memory.delete_document" => Some(
            async move {
                let payload: crate::openhuman::memory::DeleteDocumentRequest =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::memory_delete_document(payload).await?)
            }
            .await,
        ),

        "memory.query_namespace" => Some(
            async move {
                let payload: crate::openhuman::memory::QueryNamespaceRequest =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::memory_query_namespace(payload).await?)
            }
            .await,
        ),

        "memory.recall_context" | "memory.recall_namespace" => Some(
            async move {
                let payload: crate::openhuman::memory::RecallContextRequest = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::memory_recall_context(payload).await?)
            }
            .await,
        ),

        "memory.recall_memories" => Some(
            async move {
                let payload: crate::openhuman::memory::RecallMemoriesRequest =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::memory_recall_memories(payload).await?)
            }
            .await,
        ),

        "ai.list_memory_files" => Some(
            async move {
                let payload: crate::openhuman::memory::ListMemoryFilesRequest =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::ai_list_memory_files(payload).await?)
            }
            .await,
        ),

        "ai.read_memory_file" => Some(
            async move {
                let payload: crate::openhuman::memory::ReadMemoryFileRequest =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::ai_read_memory_file(payload).await?)
            }
            .await,
        ),

        "ai.write_memory_file" => Some(
            async move {
                let payload: crate::openhuman::memory::WriteMemoryFileRequest =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::ai_write_memory_file(payload).await?)
            }
            .await,
        ),

        "memory.namespace.list" => Some(
            async move { rpc_json(crate::openhuman::memory::rpc::namespace_list().await?) }.await,
        ),

        "memory.doc.put" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::PutDocParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::doc_put(payload).await?)
            }
            .await,
        ),

        "memory.doc.ingest" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::IngestDocParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::doc_ingest(payload).await?)
            }
            .await,
        ),

        "memory.doc.list" => Some(
            async move {
                let payload: MemoryDocListParams = parse_params(params)?;
                let namespace_params = payload.namespace.map(|namespace| {
                    crate::openhuman::memory::rpc::NamespaceOnlyParams { namespace }
                });
                rpc_json(crate::openhuman::memory::rpc::doc_list(namespace_params).await?)
            }
            .await,
        ),

        "memory.doc.delete" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::DeleteDocParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::doc_delete(payload).await?)
            }
            .await,
        ),

        "memory.context.query" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::QueryNamespaceParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::context_query(payload).await?)
            }
            .await,
        ),

        "memory.context.recall" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::RecallNamespaceParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::context_recall(payload).await?)
            }
            .await,
        ),

        "memory.kv.set" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvSetParams = parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_set(payload).await?)
            }
            .await,
        ),

        "memory.kv.get" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvGetDeleteParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_get(payload).await?)
            }
            .await,
        ),

        "memory.kv.delete" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::KvGetDeleteParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_delete(payload).await?)
            }
            .await,
        ),

        "memory.kv.list_namespace" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::NamespaceOnlyParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::kv_list_namespace(payload).await?)
            }
            .await,
        ),

        "memory.graph.upsert" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::GraphUpsertParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::graph_upsert(payload).await?)
            }
            .await,
        ),

        "memory.graph.query" => Some(
            async move {
                let payload: crate::openhuman::memory::rpc::GraphQueryParams =
                    parse_params(params)?;
                rpc_json(crate::openhuman::memory::rpc::graph_query(payload).await?)
            }
            .await,
        ),

        "openhuman.security_policy_info" => Some(rpc_json(
            crate::openhuman::security::rpc::security_policy_info(),
        )),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::try_dispatch;

    /// Verify that the dispatcher recognises `memory.doc.ingest` (returns `Some`).
    /// The inner handler will fail because no memory client is initialised,
    /// but the route being present (not `None`) is what we need to assert.
    #[tokio::test]
    async fn dispatch_routes_memory_doc_ingest() {
        let params = json!({
            "namespace": "test",
            "key": "k1",
            "title": "Title",
            "content": "body"
        });
        let result = try_dispatch("memory.doc.ingest", params).await;
        assert!(
            result.is_some(),
            "memory.doc.ingest should be routed by dispatch"
        );
    }

    /// Verify that `memory.graph.query` is routed.
    #[tokio::test]
    async fn dispatch_routes_memory_graph_query() {
        let params = json!({});
        let result = try_dispatch("memory.graph.query", params).await;
        assert!(
            result.is_some(),
            "memory.graph.query should be routed by dispatch"
        );
    }

    /// Unknown methods must return `None` so callers can fall through.
    #[tokio::test]
    async fn dispatch_returns_none_for_unknown_method() {
        let result = try_dispatch("nonexistent.method", json!({})).await;
        assert!(
            result.is_none(),
            "unknown methods should return None"
        );
    }

    /// Verify that params deserialization errors surface as `Some(Err(...))`.
    #[tokio::test]
    async fn dispatch_memory_doc_ingest_rejects_invalid_params() {
        // Missing required fields → should be Some(Err)
        let result = try_dispatch("memory.doc.ingest", json!({})).await;
        assert!(result.is_some());
        let inner = result.unwrap();
        assert!(
            inner.is_err(),
            "missing required fields should produce a deserialization error"
        );
        let err_msg = inner.unwrap_err();
        assert!(
            err_msg.contains("invalid params"),
            "error should mention invalid params, got: {err_msg}"
        );
    }
}
