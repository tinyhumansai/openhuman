//! `memory_chunk_context` — expand a chunk with its neighbors from the same source.
//!
//! Given a chunk_id (from memory_vector_search, memory_store_raw_chunks, etc.),
//! returns the chunk's content plus surrounding chunks from the same source,
//! ordered by timestamp. Lets the agent see the full conversation/document flow.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use crate::openhuman::memory::api::provider::{ChunkQuery, MemoryProvider};
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryChunkContextTool;

#[derive(Debug, Deserialize)]
struct Args {
    chunk_id: String,
    #[serde(default = "default_window")]
    window: usize,
}

fn default_window() -> usize {
    2
}

#[async_trait]
impl Tool for MemoryChunkContextTool {
    fn name(&self) -> &str {
        "memory_chunk_context"
    }

    fn description(&self) -> &str {
        "Expand a chunk with its neighbors from the same source. Given a \
         chunk_id from a prior search, returns the surrounding chunks in \
         timestamp order — showing the full conversation/document context \
         around a match."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["chunk_id"],
            "properties": {
                "chunk_id": {
                    "type": "string",
                    "description": "ID of the chunk to retrieve context for (from a prior search result)."
                },
                "window": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "description": "Number of neighboring chunks to include before and after (default 2)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_chunk_context: {e}"))?;

        if parsed.chunk_id.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "memory_chunk_context: chunk_id cannot be empty"
            ));
        }

        let window = parsed.window.clamp(1, 5);

        log::debug!(
            "[tool][memory_chunk_context] chunk_id={} window={}",
            parsed.chunk_id,
            window,
        );

        // Chunks are read through the bound driver rather than by opening the
        // store in this process — see the note in `vector_search.rs`.
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_chunk_context: {e}"))?;
        let chunk_reader = guard.as_chunks().ok_or_else(|| {
            anyhow::anyhow!("memory_chunk_context: memory driver does not support the chunk family")
        })?;

        // Look up the target chunk directly by ID
        let target = chunk_reader
            .get_chunk(&parsed.chunk_id)
            .await
            .map_err(|e| anyhow::anyhow!("memory_chunk_context: get_chunk failed: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("memory_chunk_context: chunk_id not found"))?;

        let source_id = target.metadata.source_id.clone();
        let source_kind = target.metadata.source_kind;

        // Per-profile memory-source gate: if the target chunk belongs to a
        // source the active profile didn't allow, surface nothing (its window
        // shares the same source). Non-source chunks always pass.
        if !crate::openhuman::memory::source_scope::chunk_source_allowed(
            &target.metadata.tags,
            &source_id,
        ) {
            return Ok(ToolResult::success(
                "Chunk is from a memory source not available to the active agent profile.",
            ));
        }

        // Get all chunks from the same source, ordered by timestamp. The
        // source-scope gate also applies here (the target was already checked
        // above; this keeps the window consistent). None = unrestricted.
        let source_query = ChunkQuery {
            source_kind: Some(source_kind),
            source_id: Some(source_id.clone()),
            limit: Some(500),
            ..Default::default()
        };
        let mut source_chunks = chunk_reader
            .list_chunks(&source_query, None)
            .await
            .map_err(|e| anyhow::anyhow!("memory_chunk_context: source query failed: {e}"))?;

        // Sort by seq_in_source (ascending) for natural reading order
        source_chunks.sort_by_key(|c| c.seq_in_source);

        // Find the target's position
        let target_pos = source_chunks
            .iter()
            .position(|c| c.id == parsed.chunk_id)
            .ok_or_else(|| anyhow::anyhow!(
                "memory_chunk_context: target chunk not found in source (source may have >500 chunks)"
            ))?;

        // Compute window bounds
        let start = target_pos.saturating_sub(window);
        let end = (target_pos + window + 1).min(source_chunks.len());
        let window_chunks = &source_chunks[start..end];

        let mut output = format!(
            "Source: {}:{} ({} total chunks)\n\
             Showing chunks {}-{} (target at position {}):\n\n",
            source_kind.as_str(),
            source_id,
            source_chunks.len(),
            start,
            end - 1,
            target_pos,
        );

        for (i, chunk) in window_chunks.iter().enumerate() {
            let abs_pos = start + i;
            let marker = if abs_pos == target_pos { " <<<" } else { "" };
            let _ = writeln!(
                output,
                "--- [seq={} | {}]{} ---\n{}",
                chunk.seq_in_source,
                chunk.metadata.timestamp.format("%Y-%m-%d %H:%M"),
                marker,
                chunk.content.trim(),
            );
        }

        log::debug!(
            "[tool][memory_chunk_context] returning {} chunks from source {}",
            window_chunks.len(),
            source_id,
        );

        Ok(ToolResult::success(output))
    }
}
