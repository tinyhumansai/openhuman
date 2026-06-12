//! Memory tree ingest logic for `ArchivistHook` — pipes closed segment prose
//! into the memory tree as `source_id="conversations:agent"`.

use super::helpers::strip_tool_calls_from_response;
use super::types::ArchivistHook;
use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline;
use crate::openhuman::memory_store::fts5;
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};

impl ArchivistHook {
    /// Pipe a closed segment's raw prose turns into the memory tree as
    /// `source_id="conversations:agent"`.
    ///
    /// **Design contract (Phase 2):**
    /// - ONE ingest per segment (not per turn) — the batch boundary is the
    ///   segment, so all turns land as a single ChatBatch.
    /// - RAW PROSE only — the LLM recap (summary) is explicitly NOT ingested.
    ///   The tree must build its own summaries from evidence (raw turns);
    ///   feeding a summary-of-a-summary violates the evidence-vs-interpretation
    ///   policy.
    /// - `source_id = "conversations:agent"` is a CONSTANT — a single shared
    ///   tree source for all agent chat sessions (never per-session or per-segment).
    /// - Tool-call JSON is stripped from assistant entries so structured
    ///   payloads do not reach the tree (memory ingestion policy).
    /// - Provenance is stamped on each `ChatMessage.source_ref` as
    ///   `agent://session/{session_id}/segment/{segment_id}#ep{start}-{end}`
    ///   so tree leaves can be traced back to episodic rows for drill-down and
    ///   deduplication.
    ///
    /// Failures are logged and swallowed; the episodic write is the source of
    /// truth.
    pub(super) async fn pipe_segment_to_tree(
        &self,
        config: &Config,
        segment: &crate::openhuman::memory_store::segments::ConversationSegment,
        session_id: &str,
        entries: &[&fts5::EpisodicEntry],
    ) {
        use chrono::{TimeZone, Utc};

        // Collect the episodic id span for provenance stamping.
        // start_episodic_id comes from the segment record (set at creation);
        // end_episodic_id is the latest turn id (may be None if only one turn).
        let start_ep = segment.start_episodic_id;
        let end_ep = segment.end_episodic_id.unwrap_or(start_ep);
        let segment_id = &segment.segment_id;

        // The provenance URI embeds session + segment + episodic id span so
        // tree leaves can be traced back to episodic_log rows without a
        // full-text scan.
        let provenance =
            format!("agent://session/{session_id}/segment/{segment_id}#ep{start_ep}-{end_ep}");

        // Build one ChatMessage per episodic entry (user + assistant; skip
        // empties). Tool-call JSON is stripped from assistant content so only
        // prose flows into the tree.
        let messages: Vec<ChatMessage> = entries
            .iter()
            .filter_map(|e| {
                let raw_text = if e.role == "assistant" {
                    strip_tool_calls_from_response(&e.content)
                } else {
                    e.content.clone()
                };
                // Strip `[IMAGE:<base64>]` attachment markers so images never
                // enter episodic memory ingestion — otherwise the base64 is
                // chunked, embedded (garbage + Voyage size errors), and fed to
                // the extract LLM (#3205). `parse_image_markers` returns the
                // marker-free prose, already trimmed; the image itself isn't
                // useful memory text. An image-only turn collapses to empty and
                // is skipped by the guard below.
                let (text, _image_refs) =
                    crate::openhuman::agent::multimodal::parse_image_markers(&raw_text);
                if text.is_empty() {
                    return None;
                }

                // Convert the f64 Unix timestamp to DateTime<Utc>.
                let secs = e.timestamp as i64;
                let nanos = ((e.timestamp.fract()) * 1e9) as u32;
                let ts = Utc
                    .timestamp_opt(secs, nanos.min(999_999_999))
                    .single()
                    .unwrap_or_else(Utc::now);

                Some(ChatMessage {
                    author: e.role.clone(),
                    timestamp: ts,
                    text,
                    source_ref: Some(provenance.clone()),
                })
            })
            .collect();

        if messages.is_empty() {
            tracing::debug!(
                "[archivist] pipe_segment_to_tree: no prose messages in segment={segment_id} — skipping"
            );
            return;
        }

        let batch = ChatBatch {
            platform: "agent".into(),
            // channel_label carries session_id for human-readable context.
            channel_label: session_id.to_string(),
            messages,
        };

        // `source_id` is intentionally a CONSTANT — all agent sessions share
        // one tree source so cross-session summarisation sees the full history.
        let source_id = "conversations:agent";
        // `owner` scopes the memory to the session; `tags` enable filtering.
        let owner = session_id;
        let tags = vec!["agent_chat".to_string()];

        tracing::debug!(
            "[archivist] tree ingest start: source_id={source_id} session={session_id} \
             segment={segment_id} ep_span={start_ep}-{end_ep} provenance={provenance}"
        );

        match ingest_pipeline::ingest_chat(config, source_id, owner, tags, batch).await {
            Ok(result) => {
                tracing::debug!(
                    "[archivist] tree ingest ok: source_id={source_id} \
                     session={session_id} segment={segment_id} \
                     chunks_written={} provenance={provenance}",
                    result.chunks_written
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[archivist] tree ingest failed (non-fatal): source_id={source_id} \
                     session={session_id} segment={segment_id} error={e}"
                );
            }
        }
    }
}
