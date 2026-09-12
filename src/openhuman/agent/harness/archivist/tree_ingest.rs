//! Memory tree ingest logic for `ArchivistHook` — pipes closed segment prose
//! into the memory tree as `source_id="conversations:agent"`.

use super::helpers::strip_tool_calls_from_response;
use super::types::ArchivistHook;
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::chunks::{DataSource, SourceRef};
use crate::openhuman::memory::api::provider::types::IngestItem;
use crate::openhuman::memory::api::provider::{ConversationSegment, EpisodicTurn};
use crate::openhuman::memory::api::types::MemoryTaint;

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
        segment: &ConversationSegment,
        session_id: &str,
        entries: &[&EpisodicTurn],
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
        let messages = entries
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

                // One IngestItem per prose turn. The widened attribution trio
                // exists for exactly this batch shape: `owner` is the session
                // the memory belongs to, `author` is the speaking role, the
                // label is human-readable while `source_id` stays the constant
                // dedupe key, and `platform: "agent"` preserves the string
                // every previously-stored chunk carries.
                Some(IngestItem {
                    namespace: None,
                    source: DataSource::Conversation,
                    source_id: "conversations:agent".to_string(),
                    owner: session_id.to_string(),
                    source_ref: Some(SourceRef::new(provenance.clone())),
                    content: text,
                    mime: Some("text/plain".into()),
                    timestamp: Some(ts),
                    tags: vec!["agent_chat".to_string()],
                    taint: MemoryTaint::Internal,
                    path_scope: None,
                    author: Some(e.role.clone()),
                    channel_label: Some(session_id.to_string()),
                    // Mail-only fields; this source is not mail, and the contract
                    // documents empty/absent as the same statement as "not mail".
                    to: Vec::new(),
                    cc: Vec::new(),
                    subject: None,
                    list_unsubscribe: None,
                    platform: Some("agent".into()),
                })
            })
            .collect::<Vec<IngestItem>>();

        if messages.is_empty() {
            tracing::debug!(
                "[archivist] pipe_segment_to_tree: no prose messages in segment={segment_id} — skipping"
            );
            return;
        }

        let source_id = "conversations:agent";
        tracing::debug!(
            "[archivist] tree ingest start: source_id={source_id} session={session_id} \
             segment={segment_id} ep_span={start_ep}-{end_ep} provenance={provenance}"
        );

        let Some(ingest) = self.provider.as_deref().and_then(|p| p.as_ingest()) else {
            tracing::debug!(
                "[archivist] tree ingest skipped: driver serves no Ingest family \
                 segment={segment_id}"
            );
            return;
        };
        // `config` gates this path (chat_to_tree_enabled) and sizes nothing
        // here any more — the driver owns chunking and extraction.
        let _ = config;

        let ingest_result = ingest.ingest_chat(messages).await;

        match ingest_result {
            Ok(result) => {
                tracing::debug!(
                    "[archivist] tree ingest ok: source_id={source_id} \
                     session={session_id} segment={segment_id} \
                     chunks_written={} provenance={provenance}",
                    result.written
                );
            }
            Err(e) => {
                // Corruption is not non-fatal (openhuman#5820): a malformed
                // chunk store fails every segment identically, and this warn
                // was one of the paths that let it run silently for 34
                // minutes. The driver's engine owns the recovery (its queue
                // worker quarantines + rebuilds within one poll interval);
                // this side's job is to escalate visibility — log at ERROR and
                // put a durable notice in front of the user, once per process
                // rather than once per segment. The engine's own quarantine
                // publishes the same `memory_store_corrupt` kind afterwards;
                // that is not a second entry, because the notice store keys on
                // the descriptor id (kind + scope) and refreshes the existing
                // one. This early notice is what covers damage the queue worker
                // never walks. The episodic write above is still the source of
                // truth either way, so the turn itself is never failed from
                // here.
                let rendered = e.to_string();
                if crate::openhuman::memory::tree::health::user_error::is_corrupt_store_error(
                    &rendered,
                ) {
                    tracing::error!(
                        "[archivist] tree ingest hit a CORRUPT memory store: \
                         source_id={source_id} session={session_id} segment={segment_id} \
                         error={rendered}"
                    );
                    crate::openhuman::memory::tree::health::user_error::notice_corrupt_store_once(
                        "archivist tree ingest",
                    );
                } else if crate::openhuman::memory::tree::health::user_error::is_local_embedding_error(
                    &rendered,
                ) {
                    tracing::warn!(
                        "[archivist] tree ingest hit a local-model embedding failure \
                         (non-fatal): source_id={source_id} session={session_id} \
                         segment={segment_id} error={rendered}"
                    );
                    // Surface a once-per-process user notification so the
                    // frontend can prompt the user to check their local model
                    // configuration (openhuman#5867). Only fires for confirmed
                    // Ollama-specific errors; storage/bus/RPC failures are
                    // logged above and left to the existing retry/backoff path.
                    crate::openhuman::memory::tree::health::user_error::notice_local_model_unavailable_once(
                        "archivist tree ingest",
                    );
                } else {
                    tracing::warn!(
                        "[archivist] tree ingest failed (non-fatal): source_id={source_id} \
                         session={session_id} segment={segment_id} error={rendered}"
                    );
                }
            }
        }
    }
}
