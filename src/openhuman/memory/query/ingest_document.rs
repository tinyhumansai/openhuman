use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::tree::rpc;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use tinymemory_api::chunks::SourceKind;

/// The `document` ingest payload, as the driver's canonicaliser reads it.
///
/// # Why this is declared here rather than imported (#5560)
///
/// It was `tinycortex::memory::ingest::canonicalize::document::DocumentInput`,
/// and the import was the last thing keeping the engine crate named in this
/// file. The contract has no equivalent and should not: `IngestRequest::payload`
/// is deliberately a `serde_json::Value`, because the shape a payload takes is
/// owned by the *source kind*, and the set of source kinds grows without a
/// contract change. A per-kind payload type in the contract would be the
/// opposite of that.
///
/// So what crosses here is JSON, and what this struct is for is naming the
/// field set that JSON must have. Serialize-only, for the same reason: the
/// engine's type carries a `deserialize_with` that accepts epoch-millis,
/// RFC 3339, or an absent timestamp, and none of that leniency is this
/// producer's business — it always emits one shape. Reproducing the read side
/// would be a second parser with no reader.
///
/// Field for field with the engine's declaration, in the same order, with no
/// serde attributes on either side beyond the defaults, so the emitted object
/// is byte-identical. `execute_success_path_roundtrips_document_chunk` is what
/// checks that end to end: it ingests through this payload and reads the
/// resulting chunk back, so a renamed or dropped field fails as a missing
/// chunk rather than as a silent no-op.
#[derive(Serialize)]
struct DocumentInput {
    /// Provider name (e.g. `notion`, `drive`, `meeting_notes`).
    provider: String,
    /// Document title.
    title: String,
    /// Document body (markdown preferred; plain text also accepted).
    body: String,
    /// When the document was last modified at the source. Emitted as RFC 3339,
    /// which is one of the three forms the canonicaliser accepts.
    modified_at: DateTime<Utc>,
    /// Optional pointer back to source (URL, file path, Notion page id).
    source_ref: Option<String>,
}

pub struct MemoryTreeIngestDocumentTool;

#[async_trait]
impl Tool for MemoryTreeIngestDocumentTool {
    fn name(&self) -> &str {
        "memory_tree_ingest_document"
    }

    fn description(&self) -> &str {
        "Ingest a document into the memory tree for future retrieval. \
         This is the write path into the knowledge index — use it after \
         fetching web content, extracting facts, or collecting data from \
         external sources. The ingested document will be chunked, embedded, \
         and available via query_source and search_entities."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Document title (e.g. 'ROOT v6.36.12 Release Notes')."
                },
                "body": {
                    "type": "string",
                    "description": "Document body in markdown or plain text."
                },
                "source_id": {
                    "type": "string",
                    "description": "Stable source identifier (e.g. 'root_releases', 'github_root_changelog'). Re-ingesting with same source_id replaces old chunks."
                },
                "provider": {
                    "type": "string",
                    "description": "Source provider name (e.g. 'github', 'web', 'root_docs'). Defaults to 'agent'."
                },
                "source_ref": {
                    "type": "string",
                    "description": "Optional URL or pointer back to the original source."
                },
                "owner": {
                    "type": "string",
                    "description": "Optional account/user this content belongs to. Used for owner-scoped queries and attribution. Defaults to empty (unowned/agent-global)."
                }
            },
            "required": ["title", "body", "source_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] ingest_document invoked");

        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("ingest_document: missing required field `title`"))?
            .to_string();
        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("ingest_document: missing required field `body`"))?
            .to_string();
        let source_id = args
            .get("source_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("ingest_document: missing required field `source_id`"))?
            .trim()
            .to_string();
        let provider = args
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("agent")
            .to_string();
        let source_ref = args
            .get("source_ref")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let owner = args
            .get("owner")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if title.trim().is_empty() || body.trim().is_empty() || source_id.is_empty() {
            return Ok(ToolResult::error(
                "ingest_document: title, body, and source_id must be non-empty".to_string(),
            ));
        }

        let cfg = config_rpc::load_config_with_timeout().await.map_err(|e| {
            log::debug!("[tool][memory_tree] ingest_document config_load_failed err={e}");
            anyhow::anyhow!("ingest_document: load config failed: {e}")
        })?;

        let doc = DocumentInput {
            provider,
            title: title.trim().to_string(),
            body: body.trim().to_string(),
            modified_at: Utc::now(),
            source_ref,
        };

        let req = rpc::IngestRequest {
            source_kind: SourceKind::Document,
            source_id: source_id.clone(),
            owner,
            tags: vec!["agent_ingested".to_string()],
            payload: serde_json::to_value(&doc).map_err(|e| {
                log::debug!("[tool][memory_tree] ingest_document payload_serialize_failed err={e}");
                anyhow::anyhow!("ingest_document: failed to serialize payload: {e}")
            })?,
        };

        let outcome = rpc::ingest_rpc(&cfg, req).await.map_err(|e| {
            log::debug!(
                "[tool][memory_tree] ingest_document rpc_failed source_id={source_id} err={e}"
            );
            anyhow::anyhow!("ingest_document: ingestion failed: {e}")
        })?;

        let n = outcome.value.chunks_written;
        log::info!(
            "[tool][memory_tree] ingest_document done source_id={} chunks={}",
            source_id,
            n
        );
        Ok(ToolResult::success(format!(
            "Ingested document \"{}\" as source_id={}. {} chunks created and indexed.",
            title, source_id, n
        )))
    }
}

#[cfg(test)]
#[path = "ingest_document_tests.rs"]
mod tests;
