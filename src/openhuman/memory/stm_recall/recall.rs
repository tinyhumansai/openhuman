//! Core STM recall logic — two-arm hybrid retrieval.
//!
//! Arm 1: FTS5 episodic cross-session search (keyword, high-precision gate).
//! Arm 2: Brute-force cosine over `segment_embeddings` (vector, medium gate).

use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::openhuman::memory_store::fts5;
use crate::openhuman::memory_store::fts5::EpisodicEntry;

use super::{
    COSINE_GATE, FTS5_LIMIT, MAX_EPISODIC_TURNS, MAX_SEGMENT_RECAPS, RECENCY_WINDOW_DAYS,
    RECENCY_WINDOW_MAX_SEGMENTS, TOKEN_BUDGET,
};

/// A single item in the assembled STM recall block.
#[derive(Debug, Clone)]
pub enum StmItem {
    /// A segment-level recap (compacted, from `segment_embeddings` arm).
    SegmentRecap {
        segment_id: String,
        session_id: String,
        summary: String,
        /// `start_episodic_id..=end_episodic_id` span — used for dedup.
        start_episodic_id: i64,
        end_episodic_id: Option<i64>,
        updated_at: f64,
        cosine: f32,
    },
    /// A raw episodic turn from Arm 1 (FTS5 keyword match).
    EpisodicTurn {
        id: Option<i64>,
        session_id: String,
        timestamp: f64,
        role: String,
        content: String,
    },
}

impl StmItem {
    fn timestamp(&self) -> f64 {
        match self {
            Self::SegmentRecap { updated_at, .. } => *updated_at,
            Self::EpisodicTurn { timestamp, .. } => *timestamp,
        }
    }

    fn approx_chars(&self) -> usize {
        match self {
            Self::SegmentRecap { summary, .. } => summary.len() + 60,
            Self::EpisodicTurn { content, role, .. } => content.len() + role.len() + 20,
        }
    }
}

/// Options for a single STM recall pass.
#[derive(Debug, Clone, Default)]
pub struct StmRecallOpts<'a> {
    /// The active session to exclude from all results.
    pub exclude_session: &'a str,
    /// Optional query text for Arm 1 (FTS5) and Arm 2 (embed).
    /// When `None`, Arm 1 falls back to a recency selection and Arm 2 is skipped.
    pub query: Option<&'a str>,
    /// Model signature to filter segment embeddings (e.g. `"cloud:voyage-3:1024"`).
    /// When `None`, any model signature is accepted (weaker but still useful).
    pub model_signature: Option<&'a str>,
}

/// The assembled STM recall block.
#[derive(Debug, Clone, Default)]
pub struct StmRecallBlock {
    /// Deduplicated, recency-weighted items, bounded to [`TOKEN_BUDGET`].
    pub items: Vec<StmItem>,
    /// Number of items dropped due to token-budget exhaustion.
    pub dropped_budget: usize,
    /// Number of episodic hits dropped because they fell inside a segment span (dedup).
    pub dropped_dedup: usize,
    /// Cosine arm — items retrieved before gating.
    pub cosine_candidates: usize,
    /// FTS5 arm — items retrieved before gating.
    pub fts5_candidates: usize,
}

impl StmRecallBlock {
    /// `true` when the block has no usable content.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Render the block into a markdown string suitable for injection into the
    /// system prompt or a user-turn context block.
    pub fn render(&self) -> String {
        if self.items.is_empty() {
            return String::new();
        }
        let mut out = String::from("## Recent context from other conversations\n\n");
        out.push_str(
            "The following snippets are from previous conversations in other chat threads. \
             They are provided for continuity — reference them when they are relevant to \
             the current request, but do not surface them unless asked.\n\n",
        );

        let mut seg_count = 0usize;
        let mut ep_count = 0usize;

        for item in &self.items {
            match item {
                StmItem::SegmentRecap {
                    session_id,
                    summary,
                    updated_at,
                    ..
                } => {
                    seg_count += 1;
                    let age_days = age_days_from_ts(*updated_at);
                    out.push_str(&format!(
                        "**Conversation recap** (thread `{session_id}`, ~{age_days:.0} days ago):\n{summary}\n\n"
                    ));
                }
                StmItem::EpisodicTurn {
                    session_id,
                    timestamp,
                    role,
                    content,
                    ..
                } => {
                    ep_count += 1;
                    let age_days = age_days_from_ts(*timestamp);
                    out.push_str(&format!(
                        "**{role}** (thread `{session_id}`, ~{age_days:.0} days ago): {content}\n\n"
                    ));
                }
            }
        }

        tracing::debug!(
            "[stm_recall] rendered block: {} recaps + {} episodic turns, {} dropped_dedup, {} dropped_budget",
            seg_count,
            ep_count,
            self.dropped_dedup,
            self.dropped_budget
        );
        out
    }
}

fn age_days_from_ts(ts: f64) -> f64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    (now - ts).max(0.0) / 86_400.0
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Brute-force cosine similarity between two equal-length float slices.
/// Returns 0.0 when either vector has zero magnitude (zero-padded / inert embedder).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

/// Decode a raw BLOB from `segment_embeddings.vector` into `Vec<f32>`.
///
/// A well-formed embedding blob is a whole number of little-endian `f32`s
/// (length a multiple of 4). A non-multiple-of-4 length means the blob is
/// truncated/corrupt; silently dropping the trailing bytes would yield a
/// wrong-length vector, so we reject it (empty → cosine treats it as a
/// non-match, which is the safe outcome).
fn decode_vector_blob(bytes: &[u8]) -> Vec<f32> {
    if bytes.len() % 4 != 0 {
        tracing::warn!(
            "[stm_recall] decode_vector_blob: blob length {} is not a multiple of 4 — \
             discarding malformed embedding",
            bytes.len()
        );
        return Vec::new();
    }
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

// ── Arm 2: vector search over segment_embeddings ──────────────────────────────

/// A row pulled from `segment_embeddings` joined with `conversation_segments`.
struct SegmentEmbeddingRow {
    segment_id: String,
    session_id: String,
    summary: Option<String>,
    start_episodic_id: i64,
    end_episodic_id: Option<i64>,
    updated_at: f64,
    vector: Vec<f32>,
    model_signature: String,
}

/// Load candidate segment embeddings for the cosine pass.
///
/// Applies:
/// - Recency window (last [`RECENCY_WINDOW_DAYS`] days)
/// - Exclude current `session_id`
/// - Optional `model_signature` filter
/// - Capped at [`RECENCY_WINDOW_MAX_SEGMENTS`] rows
fn load_segment_embedding_candidates(
    conn: &Arc<Mutex<Connection>>,
    exclude_session: &str,
    model_signature: Option<&str>,
    cutoff_ts: f64,
) -> anyhow::Result<Vec<SegmentEmbeddingRow>> {
    let conn = conn.lock();

    // We join `segment_embeddings` (se) with `conversation_segments` (cs)
    // to get the session_id, summary, episodic span, and recency.
    // Filter: cs.session_id != exclude_session, cs.updated_at >= cutoff_ts,
    //         cs.summary IS NOT NULL (only summarised segments have useful recaps),
    //         optionally se.model_signature = ?
    let rows: Vec<SegmentEmbeddingRow> = if let Some(sig) = model_signature {
        let mut stmt = conn.prepare(
            "SELECT se.segment_id, cs.session_id, cs.summary,
                    cs.start_episodic_id, cs.end_episodic_id,
                    cs.updated_at, se.vector, se.model_signature
               FROM segment_embeddings AS se
               JOIN conversation_segments AS cs ON se.segment_id = cs.segment_id
              WHERE cs.session_id != ?1
                AND cs.updated_at >= ?2
                AND cs.summary IS NOT NULL
                AND se.model_signature = ?3
              ORDER BY cs.updated_at DESC
              LIMIT ?4",
        )?;
        let collected: Vec<SegmentEmbeddingRow> = stmt
            .query_map(
                params![
                    exclude_session,
                    cutoff_ts,
                    sig,
                    RECENCY_WINDOW_MAX_SEGMENTS as i64
                ],
                |row| {
                    let vector_bytes: Vec<u8> = row.get(6)?;
                    Ok(SegmentEmbeddingRow {
                        segment_id: row.get(0)?,
                        session_id: row.get(1)?,
                        summary: row.get(2)?,
                        start_episodic_id: row.get(3)?,
                        end_episodic_id: row.get(4)?,
                        updated_at: row.get(5)?,
                        vector: decode_vector_blob(&vector_bytes),
                        model_signature: row.get(7)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    } else {
        // No model filter — accept any model (weaker but useful when no sig available).
        let mut stmt = conn.prepare(
            "SELECT se.segment_id, cs.session_id, cs.summary,
                    cs.start_episodic_id, cs.end_episodic_id,
                    cs.updated_at, se.vector, se.model_signature
               FROM segment_embeddings AS se
               JOIN conversation_segments AS cs ON se.segment_id = cs.segment_id
              WHERE cs.session_id != ?1
                AND cs.updated_at >= ?2
                AND cs.summary IS NOT NULL
              ORDER BY cs.updated_at DESC
              LIMIT ?3",
        )?;
        let collected: Vec<SegmentEmbeddingRow> = stmt
            .query_map(
                params![
                    exclude_session,
                    cutoff_ts,
                    RECENCY_WINDOW_MAX_SEGMENTS as i64
                ],
                |row| {
                    let vector_bytes: Vec<u8> = row.get(6)?;
                    Ok(SegmentEmbeddingRow {
                        segment_id: row.get(0)?,
                        session_id: row.get(1)?,
                        summary: row.get(2)?,
                        start_episodic_id: row.get(3)?,
                        end_episodic_id: row.get(4)?,
                        updated_at: row.get(5)?,
                        vector: decode_vector_blob(&vector_bytes),
                        model_signature: row.get(7)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };

    tracing::debug!(
        "[stm_recall] arm2: loaded {} segment embedding candidates (model_sig={:?})",
        rows.len(),
        model_signature
    );
    Ok(rows)
}

/// Load recent episodic turns from other sessions (recency fallback for Arm 1
/// when no query is available).
fn load_recent_episodic_other_sessions(
    conn: &Arc<Mutex<Connection>>,
    exclude_session: &str,
    cutoff_ts: f64,
    limit: usize,
) -> anyhow::Result<Vec<EpisodicEntry>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT id, session_id, timestamp, role, content, lesson, tool_calls_json, cost_microdollars
           FROM episodic_log
          WHERE session_id != ?1
            AND timestamp >= ?2
          ORDER BY timestamp DESC
          LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![exclude_session, cutoff_ts, limit as i64], |row| {
            Ok(EpisodicEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                timestamp: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                lesson: row.get(5)?,
                tool_calls_json: row.get(6)?,
                cost_microdollars: row.get::<_, i64>(7)? as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    tracing::debug!(
        "[stm_recall] arm1 recency fallback: loaded {} episodic turns from other sessions",
        rows.len()
    );
    Ok(rows)
}

/// Main entry point: run the two-arm STM recall and return an assembled block.
///
/// When `opts.query` is `None` (preemptive/session-start case):
/// - Arm 1 uses a recency selection of recent other-session episodic turns.
/// - Arm 2 is skipped (no query vector to embed; caller may pre-compute one).
///
/// When `opts.query` is `Some(q)`:
/// - Arm 1 runs FTS5 cross-session search with `q` and the high-precision gate.
/// - Arm 2 runs brute-force cosine against any pre-computed `query_embedding`.
///
/// `query_embedding` is the embedding of `opts.query` (when provided).
/// The caller is responsible for producing it (avoids a blocking embed call here).
/// If `None`, Arm 2 is skipped regardless of `opts.query`.
pub fn stm_recall(
    conn: &Arc<Mutex<Connection>>,
    opts: &StmRecallOpts<'_>,
    query_embedding: Option<&[f32]>,
) -> anyhow::Result<StmRecallBlock> {
    let cutoff_ts = now_secs() - RECENCY_WINDOW_DAYS * 86_400.0;
    let mut block = StmRecallBlock::default();

    tracing::debug!(
        "[stm_recall] starting recall exclude_session={} has_query={} has_embedding={} recency_days={}",
        opts.exclude_session,
        opts.query.is_some(),
        query_embedding.is_some(),
        RECENCY_WINDOW_DAYS
    );

    // ── Arm 2: vector search over segment_embeddings ──────────────────────────
    let mut segment_items: Vec<StmItem> = Vec::new();
    let mut segment_spans: Vec<(i64, Option<i64>)> = Vec::new(); // for dedup

    if let Some(q_emb) = query_embedding {
        if !q_emb.is_empty() {
            let candidates = load_segment_embedding_candidates(
                conn,
                opts.exclude_session,
                opts.model_signature,
                cutoff_ts,
            )?;
            block.cosine_candidates = candidates.len();

            let mut scored: Vec<(f32, SegmentEmbeddingRow)> = candidates
                .into_iter()
                .filter_map(|row| {
                    if row.vector.is_empty() {
                        tracing::debug!(
                            "[stm_recall] arm2: skipping segment {} — zero-length vector (inert embedder?)",
                            row.segment_id
                        );
                        return None;
                    }
                    let cos = cosine_similarity(q_emb, &row.vector);
                    tracing::debug!(
                        "[stm_recall] arm2: segment={} session={} cosine={:.3} gate={}",
                        row.segment_id,
                        row.session_id,
                        cos,
                        COSINE_GATE
                    );
                    if cos >= COSINE_GATE {
                        Some((cos, row))
                    } else {
                        None
                    }
                })
                .collect();

            // Sort descending by cosine then recency
            scored.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(
                        b.1.updated_at
                            .partial_cmp(&a.1.updated_at)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
            });

            for (cos, row) in scored.into_iter().take(MAX_SEGMENT_RECAPS) {
                let summary = match row.summary {
                    Some(ref s) if !s.trim().is_empty() => s.clone(),
                    _ => continue,
                };
                tracing::debug!(
                    "[stm_recall] arm2: accepting segment={} session={} cosine={:.3} model={}",
                    row.segment_id,
                    row.session_id,
                    cos,
                    row.model_signature
                );
                segment_spans.push((row.start_episodic_id, row.end_episodic_id));
                segment_items.push(StmItem::SegmentRecap {
                    segment_id: row.segment_id,
                    session_id: row.session_id,
                    summary,
                    start_episodic_id: row.start_episodic_id,
                    end_episodic_id: row.end_episodic_id,
                    updated_at: row.updated_at,
                    cosine: cos,
                });
            }
            tracing::debug!(
                "[stm_recall] arm2: {} recaps accepted after cosine gate",
                segment_items.len()
            );
        }
    }

    // ── Arm 1: FTS5 or recency episodic ──────────────────────────────────────
    let mut episodic_items: Vec<StmItem> = Vec::new();

    let raw_episodic: Vec<EpisodicEntry> = if let Some(query) = opts.query {
        // Keyword search path
        let hits = fts5::episodic_cross_session_search(
            conn,
            query,
            FTS5_LIMIT,
            Some(opts.exclude_session),
        )?;
        block.fts5_candidates = hits.len();
        tracing::debug!(
            "[stm_recall] arm1 FTS5: {} hits for query (exclude={})",
            hits.len(),
            opts.exclude_session
        );
        hits
    } else {
        // Recency fallback — no query; pull most-recent turns from other sessions
        let hits =
            load_recent_episodic_other_sessions(conn, opts.exclude_session, cutoff_ts, FTS5_LIMIT)?;
        block.fts5_candidates = hits.len();
        hits
    };

    // Dedup: drop episodic rows whose ID falls within any accepted segment's span.
    for entry in raw_episodic {
        let entry_id = entry.id.unwrap_or(-1);

        // Check if this episodic entry is covered by any accepted segment span.
        let covered = segment_spans
            .iter()
            .any(|(start, end)| entry_id >= *start && end.map_or(false, |e| entry_id <= e));

        if covered {
            block.dropped_dedup += 1;
            tracing::debug!(
                "[stm_recall] arm1: dropping episodic id={} — covered by segment span",
                entry_id
            );
            continue;
        }

        // Recency window applies consistently — including keyword/FTS mode.
        // FTS5 keyword search is not time-bounded, so without this an
        // older-than-window episodic hit could leak into STM. The recency
        // window IS the STM/LTM boundary and must always hold.
        if entry.timestamp < cutoff_ts {
            continue;
        }

        episodic_items.push(StmItem::EpisodicTurn {
            id: entry.id,
            session_id: entry.session_id,
            timestamp: entry.timestamp,
            role: entry.role,
            content: entry.content,
        });
    }

    tracing::debug!(
        "[stm_recall] arm1: {} episodic items after dedup (dropped_dedup={})",
        episodic_items.len(),
        block.dropped_dedup
    );

    // ── Merge + cap ───────────────────────────────────────────────────────────
    // Recency-weight: sort each arm descending by timestamp. Interleave by picking
    // the most-recent item across both arms.
    segment_items.sort_by(|a, b| {
        b.timestamp()
            .partial_cmp(&a.timestamp())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    episodic_items.sort_by(|a, b| {
        b.timestamp()
            .partial_cmp(&a.timestamp())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Apply top-k caps before interleave
    let seg_capped: Vec<StmItem> = segment_items.into_iter().take(MAX_SEGMENT_RECAPS).collect();
    let ep_capped: Vec<StmItem> = episodic_items
        .into_iter()
        .take(MAX_EPISODIC_TURNS)
        .collect();

    // Interleave: recency-first merge
    let mut seg_iter = seg_capped.into_iter().peekable();
    let mut ep_iter = ep_capped.into_iter().peekable();
    let mut merged: Vec<StmItem> = Vec::new();

    loop {
        match (seg_iter.peek(), ep_iter.peek()) {
            (None, None) => break,
            (Some(_), None) => {
                merged.extend(seg_iter.by_ref());
                break;
            }
            (None, Some(_)) => {
                merged.extend(ep_iter.by_ref());
                break;
            }
            (Some(s), Some(e)) => {
                if s.timestamp() >= e.timestamp() {
                    merged.push(seg_iter.next().expect("peek confirmed Some"));
                } else {
                    merged.push(ep_iter.next().expect("peek confirmed Some"));
                }
            }
        }
    }

    // Apply token budget
    let mut used_chars = 0usize;
    let mut final_items: Vec<StmItem> = Vec::new();
    let mut dropped_budget = 0usize;

    for item in merged {
        let chars = item.approx_chars();
        if used_chars + chars > TOKEN_BUDGET {
            dropped_budget += 1;
            tracing::debug!(
                "[stm_recall] budget: dropping item (would exceed {TOKEN_BUDGET} chars)"
            );
            continue;
        }
        used_chars += chars;
        final_items.push(item);
    }

    tracing::debug!(
        "[stm_recall] final block: {} items, ~{} chars, {} dropped_budget, {} dropped_dedup, {} cosine_candidates, {} fts5_candidates",
        final_items.len(),
        used_chars,
        dropped_budget,
        block.dropped_dedup,
        block.cosine_candidates,
        block.fts5_candidates
    );

    block.items = final_items;
    block.dropped_budget = dropped_budget;
    Ok(block)
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "recall_tests.rs"]
mod tests;
