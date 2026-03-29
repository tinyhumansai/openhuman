use serde::Deserialize;

use crate::core_server::helpers::{parse_params, rpc_invocation_from_outcome};
use crate::core_server::types::InvocationResult;
use crate::openhuman::memory::rpc as memory_rpc;

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "memory.namespace.list" => Some(
            async move { rpc_invocation_from_outcome(memory_rpc::namespace_list().await?) }.await,
        ),

        "memory.doc.put" => Some(
            async move {
                let payload: memory_rpc::PutDocParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::doc_put(payload).await?)
            }
            .await,
        ),

        "memory.doc.list" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct DocListParams {
                    namespace: Option<String>,
                }
                let payload: DocListParams = parse_params(params)?;
                let namespace_params = payload
                    .namespace
                    .map(|namespace| memory_rpc::NamespaceOnlyParams { namespace });
                rpc_invocation_from_outcome(memory_rpc::doc_list(namespace_params).await?)
            }
            .await,
        ),

        "memory.doc.delete" => Some(
            async move {
                let payload: memory_rpc::DeleteDocParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::doc_delete(payload).await?)
            }
            .await,
        ),

        "memory.context.query" => Some(
            async move {
                let payload: memory_rpc::QueryNamespaceParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::context_query(payload).await?)
            }
            .await,
        ),

        "memory.context.recall" => Some(
            async move {
                let payload: memory_rpc::RecallNamespaceParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::context_recall(payload).await?)
            }
            .await,
        ),

        "memory.kv.set" => Some(
            async move {
                let payload: memory_rpc::KvSetParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::kv_set(payload).await?)
            }
            .await,
        ),

        "memory.kv.get" => Some(
            async move {
                let payload: memory_rpc::KvGetDeleteParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::kv_get(payload).await?)
            }
            .await,
        ),

        "memory.kv.delete" => Some(
            async move {
                let payload: memory_rpc::KvGetDeleteParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::kv_delete(payload).await?)
            }
            .await,
        ),

        "memory.kv.list_namespace" => Some(
            async move {
                let payload: memory_rpc::NamespaceOnlyParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::kv_list_namespace(payload).await?)
            }
            .await,
        ),

        "memory.graph.upsert" => Some(
            async move {
                let payload: memory_rpc::GraphUpsertParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::graph_upsert(payload).await?)
            }
            .await,
        ),

        "memory.graph.query" => Some(
            async move {
                let payload: memory_rpc::GraphQueryParams = parse_params(params)?;
                rpc_invocation_from_outcome(memory_rpc::graph_query(payload).await?)
            }
            .await,
        ),

        _ => None,
    }
}
