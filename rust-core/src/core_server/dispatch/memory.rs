use serde::Deserialize;

use crate::core_server::helpers::{
    extract_namespaces_from_documents, filter_documents_payload_by_namespace, parse_params,
};
use crate::core_server::types::InvocationResult;

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "memory.init" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryInitParams {
                    #[allow(dead_code)]
                    jwt_token: Option<String>,
                }

                let _payload: MemoryInitParams = parse_params(params)?;
                let _client = crate::memory::MemoryClient::new_local()?;
                InvocationResult::ok(true)
            }
            .await,
        ),

        "memory.list_documents" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryListDocumentsParams {
                    namespace: Option<String>,
                }

                let payload: MemoryListDocumentsParams = parse_params(params)?;
                let client = crate::memory::MemoryClient::new_local()?;
                let docs = client.list_documents().await?;
                let filtered = payload
                    .namespace
                    .as_deref()
                    .map(str::trim)
                    .filter(|ns| !ns.is_empty())
                    .map(|ns| filter_documents_payload_by_namespace(docs.clone(), ns))
                    .unwrap_or(docs);
                InvocationResult::ok(filtered)
            }
            .await,
        ),

        "memory.list_namespaces" => Some(
            async move {
                let client = crate::memory::MemoryClient::new_local()?;
                let docs = client.list_documents().await?;
                InvocationResult::ok(extract_namespaces_from_documents(&docs))
            }
            .await,
        ),

        "memory.delete_document" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryDeleteDocumentParams {
                    document_id: String,
                    namespace: String,
                }

                let payload: MemoryDeleteDocumentParams = parse_params(params)?;
                let client = crate::memory::MemoryClient::new_local()?;
                let result = client
                    .delete_document(&payload.document_id, &payload.namespace)
                    .await?;
                InvocationResult::ok(result)
            }
            .await,
        ),

        "memory.query_namespace" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryQueryNamespaceParams {
                    namespace: String,
                    query: String,
                    max_chunks: Option<u32>,
                }

                let payload: MemoryQueryNamespaceParams = parse_params(params)?;
                let client = crate::memory::MemoryClient::new_local()?;
                let result = client
                    .query_namespace_context(
                        &payload.namespace,
                        &payload.query,
                        payload.max_chunks.unwrap_or(10),
                    )
                    .await?;
                InvocationResult::ok(result)
            }
            .await,
        ),

        "memory.recall_namespace" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryRecallNamespaceParams {
                    namespace: String,
                    max_chunks: Option<u32>,
                }

                let payload: MemoryRecallNamespaceParams = parse_params(params)?;
                let client = crate::memory::MemoryClient::new_local()?;
                let result = client
                    .recall_namespace_context(&payload.namespace, payload.max_chunks.unwrap_or(10))
                    .await?;
                InvocationResult::ok(result)
            }
            .await,
        ),

        _ => None,
    }
}
