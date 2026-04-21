//! Ingest orchestrator: canonicalise → chunk → persist (Phase 1 / #707).
//!
//! Consumers call one `ingest_*` function per source kind. Each returns an
//! [`IngestResult`] so the RPC layer can report how many chunks landed.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::canonicalize::{
    chat::{self, ChatBatch},
    document::{self, DocumentInput},
    email::{self, EmailThread},
};
use crate::openhuman::memory::tree::chunker::{chunk_markdown, ChunkerInput, ChunkerOptions};
use crate::openhuman::memory::tree::store;
use crate::openhuman::memory::tree::types::Chunk;

/// Outcome of one ingest call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestResult {
    pub source_id: String,
    pub chunks_written: usize,
    pub chunk_ids: Vec<String>,
}

impl IngestResult {
    fn from_chunks(source_id: String, chunks: &[Chunk], chunks_written: usize) -> Self {
        Self {
            source_id,
            chunks_written,
            chunk_ids: chunks.iter().map(|c| c.id.clone()).collect(),
        }
    }
}

/// Ingest a batch of chat messages scoped to one channel/group.
pub fn ingest_chat(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    batch: ChatBatch,
) -> Result<IngestResult> {
    log::debug!(
        "[memory_tree::ingest] chat source_id={} msg_count={}",
        source_id,
        batch.messages.len()
    );
    let canonical =
        match chat::canonicalise(source_id, owner, &tags, batch).map_err(anyhow::Error::msg)? {
            Some(c) => c,
            None => {
                return Ok(IngestResult {
                    source_id: source_id.to_string(),
                    chunks_written: 0,
                    chunk_ids: Vec::new(),
                });
            }
        };
    persist(config, source_id, canonical)
}

/// Ingest a single email thread.
pub fn ingest_email(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    thread: EmailThread,
) -> Result<IngestResult> {
    log::debug!(
        "[memory_tree::ingest] email source_id={} msg_count={}",
        source_id,
        thread.messages.len()
    );
    let canonical =
        match email::canonicalise(source_id, owner, &tags, thread).map_err(anyhow::Error::msg)? {
            Some(c) => c,
            None => {
                return Ok(IngestResult {
                    source_id: source_id.to_string(),
                    chunks_written: 0,
                    chunk_ids: Vec::new(),
                });
            }
        };
    persist(config, source_id, canonical)
}

/// Ingest a single standalone document.
pub fn ingest_document(
    config: &Config,
    source_id: &str,
    owner: &str,
    tags: Vec<String>,
    doc: DocumentInput,
) -> Result<IngestResult> {
    let title_len = doc.title.chars().count();
    log::debug!(
        "[memory_tree::ingest] document source_id={} has_title={} title_len={}",
        source_id,
        !doc.title.trim().is_empty(),
        title_len
    );
    let canonical =
        match document::canonicalise(source_id, owner, &tags, doc).map_err(anyhow::Error::msg)? {
            Some(c) => c,
            None => {
                return Ok(IngestResult {
                    source_id: source_id.to_string(),
                    chunks_written: 0,
                    chunk_ids: Vec::new(),
                });
            }
        };
    persist(config, source_id, canonical)
}

fn persist(
    config: &Config,
    source_id: &str,
    canonical: crate::openhuman::memory::tree::canonicalize::CanonicalisedSource,
) -> Result<IngestResult> {
    let input = ChunkerInput {
        source_kind: canonical.metadata.source_kind,
        source_id: source_id.to_string(),
        markdown: canonical.markdown,
        metadata: canonical.metadata,
    };
    let chunks = chunk_markdown(&input, &ChunkerOptions::default());
    let written = store::upsert_chunks(config, &chunks)?;
    log::debug!(
        "[memory_tree::ingest] persisted source_id={} chunks={}",
        source_id,
        written
    );
    Ok(IngestResult::from_chunks(
        source_id.to_string(),
        &chunks,
        written,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::canonicalize::chat::ChatMessage;
    use crate::openhuman::memory::tree::store::{count_chunks, list_chunks, ListChunksQuery};
    use crate::openhuman::memory::tree::types::SourceKind;
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    #[test]
    fn ingest_chat_writes_chunks() {
        let (_tmp, cfg) = test_config();
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![
                ChatMessage {
                    author: "alice".into(),
                    timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                    text: "hello".into(),
                    source_ref: Some("slack://m1".into()),
                },
                ChatMessage {
                    author: "bob".into(),
                    timestamp: Utc.timestamp_millis_opt(1_700_000_010_000).unwrap(),
                    text: "world".into(),
                    source_ref: None,
                },
            ],
        };
        let out = ingest_chat(&cfg, "slack:#eng", "alice", vec![], batch).unwrap();
        assert_eq!(out.chunks_written, 1);
        assert_eq!(count_chunks(&cfg).unwrap(), 1);
        let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
        assert_eq!(rows[0].metadata.source_kind, SourceKind::Chat);
        assert_eq!(rows[0].metadata.source_id, "slack:#eng");
    }

    #[test]
    fn ingest_chat_empty_batch_is_noop() {
        let (_tmp, cfg) = test_config();
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![],
        };
        let out = ingest_chat(&cfg, "slack:#eng", "alice", vec![], batch).unwrap();
        assert_eq!(out.chunks_written, 0);
        assert_eq!(count_chunks(&cfg).unwrap(), 0);
    }

    #[test]
    fn re_ingest_is_idempotent() {
        let (_tmp, cfg) = test_config();
        let doc = DocumentInput {
            provider: "notion".into(),
            title: "Launch plan".into(),
            body: "content here".into(),
            modified_at: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            source_ref: Some("notion://page/abc".into()),
        };
        ingest_document(&cfg, "notion:abc", "alice", vec![], doc.clone()).unwrap();
        ingest_document(&cfg, "notion:abc", "alice", vec![], doc).unwrap();
        assert_eq!(count_chunks(&cfg).unwrap(), 1);
    }

    #[test]
    fn chunks_preserve_source_ref() {
        let (_tmp, cfg) = test_config();
        let doc = DocumentInput {
            provider: "notion".into(),
            title: "t".into(),
            body: "b".into(),
            modified_at: Utc::now(),
            source_ref: Some("notion://x".into()),
        };
        ingest_document(&cfg, "notion:x", "alice", vec![], doc).unwrap();
        let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
        assert_eq!(
            rows[0].metadata.source_ref.as_ref().unwrap().value,
            "notion://x"
        );
    }
}
