//! RPC handler functions for the memory tree layer.
//!
//! Public JSON-RPC surface:
//! - `openhuman.memory_tree_ingest` — one unified ingest. Caller supplies
//!   `source_kind` + generic JSON `payload` (adapter-specific). Internally
//!   dispatches to chat / email / document canonicalisers.
//! - `openhuman.memory_tree_list_chunks` — listing with filters.
//! - `openhuman.memory_tree_get_chunk` — single chunk fetch.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::canonicalize::{
    chat::ChatBatch, document::DocumentInput, email::EmailThread,
};
use crate::openhuman::memory::tree::ingest::{
    ingest_chat as do_ingest_chat, ingest_document as do_ingest_document,
    ingest_email as do_ingest_email, IngestResult,
};
use crate::openhuman::memory::tree::store::{self, ListChunksQuery};
use crate::openhuman::memory::tree::types::{Chunk, SourceKind};
use crate::rpc::RpcOutcome;

/// Unified ingest request. The `payload` shape is adapter-specific and is
/// validated inside the dispatch based on `source_kind`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestRequest {
    /// Which kind of source the payload represents.
    pub source_kind: SourceKind,
    /// Logical source id (channel/group for chat, thread for email, doc id).
    pub source_id: String,
    /// Account/user this content belongs to.
    #[serde(default)]
    pub owner: String,
    /// Optional labels/tags carried through.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Adapter-specific payload — shape matches the canonicaliser for
    /// `source_kind`:
    /// - `chat`     → [`ChatBatch`]
    /// - `email`    → [`EmailThread`]
    /// - `document` → [`DocumentInput`]
    pub payload: Value,
}

/// Unified ingest RPC handler. Dispatches on `source_kind`.
pub async fn ingest_rpc(
    config: &Config,
    req: IngestRequest,
) -> Result<RpcOutcome<IngestResult>, String> {
    let IngestRequest {
        source_kind,
        source_id,
        owner,
        tags,
        payload,
    } = req;

    log::debug!(
        "[memory_tree::rpc] ingest kind={} source_id={}",
        source_kind.as_str(),
        source_id
    );

    // Phase 2: ingest functions are async. Their scoring stage awaits the
    // extractor (cheap for regex, not-cheap for future GLiNER/LLM impls)
    // and the DB work is isolated on `spawn_blocking` inside `persist`.
    let result = match source_kind {
        SourceKind::Chat => {
            let batch: ChatBatch = serde_json::from_value(payload)
                .map_err(|e| format!("invalid chat payload: {e}"))?;
            do_ingest_chat(config, &source_id, &owner, tags, batch)
                .await
                .map_err(|e| format!("ingest: {e}"))?
        }
        SourceKind::Email => {
            let thread: EmailThread = serde_json::from_value(payload)
                .map_err(|e| format!("invalid email payload: {e}"))?;
            do_ingest_email(config, &source_id, &owner, tags, thread)
                .await
                .map_err(|e| format!("ingest: {e}"))?
        }
        SourceKind::Document => {
            let doc: DocumentInput = serde_json::from_value(payload)
                .map_err(|e| format!("invalid document payload: {e}"))?;
            do_ingest_document(config, &source_id, &owner, tags, doc)
                .await
                .map_err(|e| format!("ingest: {e}"))?
        }
    };

    Ok(RpcOutcome::single_log(
        result,
        format!(
            "memory_tree: ingest kind={} source_id={source_id}",
            source_kind.as_str()
        ),
    ))
}

/// Query shape for the `list_chunks` RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ListChunksRequest {
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub until_ms: Option<i64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListChunksResponse {
    pub chunks: Vec<Chunk>,
}

pub async fn list_chunks_rpc(
    config: &Config,
    req: ListChunksRequest,
) -> Result<RpcOutcome<ListChunksResponse>, String> {
    let query = ListChunksQuery {
        source_kind: match req.source_kind.as_deref() {
            None => None,
            Some(s) => Some(SourceKind::parse(s)?),
        },
        source_id: req.source_id,
        owner: req.owner,
        since_ms: req.since_ms,
        until_ms: req.until_ms,
        limit: req.limit,
    };
    let rows = tokio::task::spawn_blocking({
        let config = config.clone();
        move || store::list_chunks(&config, &query)
    })
    .await
    .map_err(|e| format!("list_chunks join error: {e}"))?
    .map_err(|e| format!("list_chunks: {e}"))?;

    let n = rows.len();
    Ok(RpcOutcome::single_log(
        ListChunksResponse { chunks: rows },
        format!("memory_tree: list_chunks n={n}"),
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetChunkRequest {
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetChunkResponse {
    pub chunk: Option<Chunk>,
}

pub async fn get_chunk_rpc(
    config: &Config,
    req: GetChunkRequest,
) -> Result<RpcOutcome<GetChunkResponse>, String> {
    let id = req.id.clone();
    let chunk = tokio::task::spawn_blocking({
        let config = config.clone();
        move || store::get_chunk(&config, &id)
    })
    .await
    .map_err(|e| format!("get_chunk join error: {e}"))?
    .map_err(|e| format!("get_chunk: {e}"))?;
    Ok(RpcOutcome::single_log(
        GetChunkResponse { chunk },
        format!("memory_tree: get_chunk id={}", req.id),
    ))
}
