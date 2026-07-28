//! Durable re-embedding sweep for `vector_chunks`.
//!
//! A chunk whose vector never landed — the batch embed failed and the row was
//! persisted text-only (`documents.rs`) — or whose stored vector is unusable
//! under the active embedder (a dimension the live query can never match, e.g.
//! the degenerate `dims=1` rows a partial cloud-embedding failure leaves
//! behind) is invisible to recall: the vector path skips any chunk whose
//! dimension differs from the live query embedding (`query.rs`).
//!
//! Those deficient rows persist in the table, so they are their own durable
//! work-list: the sweep re-derives the pending set from `vector_chunks` on
//! every boot. That is exactly what makes recovery survive a service restart
//! without a separate queue — the missing/degenerate vector *is* the pending
//! marker.

use rusqlite::params;

use super::UnifiedMemory;
use crate::openhuman::embeddings::retry_after::{backoff_ms_for_attempt, MAX_429_RETRIES};
use crate::openhuman::memory_tree::health::classify_embed_error;

/// A `vector_chunks` row whose embedding must be recomputed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReembedCandidate {
    pub namespace: String,
    pub document_id: String,
    pub chunk_id: String,
    pub text: String,
}

/// Tally of one [`UnifiedMemory::reembed_pending`] pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReembedSweepReport {
    /// Deficient rows the scan returned this pass (bounded by `budget`).
    pub scanned: usize,
    /// Rows given a fresh, usable vector.
    pub reembedded: usize,
    /// Rows still without a usable vector after this pass (they stay pending).
    pub failed: usize,
}

impl UnifiedMemory {
    /// Scan `vector_chunks` for rows whose vector is missing or unusable under
    /// the active embedder, newest-first, capped at `limit`.
    ///
    /// A row is pending when either holds:
    ///   * `embedding IS NULL` — the batch embed failed and the chunk was
    ///     persisted text-only.
    ///   * `dim IS NULL` or `dim <> active` — a dimension the live query can
    ///     never match, so recall drops the row. This is the signal that
    ///     catches the degenerate `dims=1` vectors a partial cloud failure
    ///     writes: they satisfy a naive "has an embedding" check yet score
    ///     against nothing.
    ///
    /// Dimension — not the signature string — is the predicate on purpose: it
    /// is the format-agnostic, numeric truth, so this stays correct regardless
    /// of the separate signature-convention unification. Blank-text rows are
    /// excluded: they are permanently un-embeddable, not pending work, and
    /// re-queuing them would spin forever.
    pub(crate) fn scan_chunks_needing_reembed(
        &self,
        limit: usize,
    ) -> Result<Vec<ReembedCandidate>, String> {
        let active_dim = self.embedder.dimensions() as i64;
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT namespace, document_id, chunk_id, text
                   FROM vector_chunks
                  WHERE (embedding IS NULL OR dim IS NULL OR dim <> ?1)
                    AND text IS NOT NULL
                    AND trim(text) <> ''
                  ORDER BY updated_at DESC, chunk_id ASC
                  LIMIT ?2",
            )
            .map_err(|e| format!("prepare scan_chunks_needing_reembed: {e}"))?;
        let rows = stmt
            .query_map(params![active_dim, limit as i64], |row| {
                Ok(ReembedCandidate {
                    namespace: row.get(0)?,
                    document_id: row.get(1)?,
                    chunk_id: row.get(2)?,
                    text: row.get(3)?,
                })
            })
            .map_err(|e| format!("query scan_chunks_needing_reembed: {e}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("collect scan_chunks_needing_reembed: {e}"))
    }

    /// Re-embed up to `budget` deficient chunks (see
    /// [`scan_chunks_needing_reembed`]) in a single pass, newest-first.
    ///
    /// `budget` doubles as the batch size: the pending texts are embedded in
    /// one provider call, so a scheduler bounds provider load by calling this
    /// repeatedly with a modest budget rather than passing a huge one. A
    /// transient provider failure retries the batch with jittered exponential
    /// backoff; an unrecoverable one (auth missing, refused input) ends the
    /// pass without a tight loop — the rows stay pending in the table and the
    /// next scheduled pass retries them, so recovery survives a restart and a
    /// signed-out interval alike.
    pub(crate) async fn reembed_pending(&self, budget: usize) -> ReembedSweepReport {
        let mut report = ReembedSweepReport::default();
        if budget == 0 {
            return report;
        }
        let candidates = match self.scan_chunks_needing_reembed(budget) {
            Ok(candidates) => candidates,
            Err(error) => {
                log::warn!("[memory::reembed] scan failed: {error}");
                return report;
            }
        };
        report.scanned = candidates.len();
        if candidates.is_empty() {
            return report;
        }

        let texts: Vec<&str> = candidates.iter().map(|c| c.text.as_str()).collect();
        let vectors = match self.embed_with_backoff(&texts).await {
            Ok(vectors) => vectors,
            Err(error) => {
                let failure = classify_embed_error(&error);
                log::warn!(
                    "[memory::reembed] batch embed failed for {} chunk(s) (code={:?}, unrecoverable={}): {error:#}",
                    candidates.len(),
                    failure.code,
                    failure.is_unrecoverable(),
                );
                report.failed = candidates.len();
                return report;
            }
        };

        let signature = self.embedder.signature();
        let active_dim = self.embedder.dimensions();
        let now = Self::now_ts();
        for (candidate, vector) in candidates.iter().zip(vectors.iter()) {
            // Only a vector the live query can actually score against counts as
            // a repair. A provider that degrades mid-failure returns short
            // vectors (flo's `dims=1` rows came from exactly that); writing one
            // back would satisfy "has an embedding" while still matching
            // nothing, and — because the row stays pending — would make the
            // sweep rewrite the same row forever.
            if vector.len() != active_dim {
                log::debug!(
                    "[memory::reembed] provider returned {} dims for {}/{} (active {active_dim}) — leaving pending",
                    vector.len(),
                    candidate.namespace,
                    candidate.chunk_id
                );
                report.failed += 1;
                continue;
            }
            match self.write_chunk_embedding(candidate, vector, &signature, now) {
                Ok(()) => report.reembedded += 1,
                Err(error) => {
                    log::warn!(
                        "[memory::reembed] write-back failed for {}/{}: {error}",
                        candidate.namespace,
                        candidate.chunk_id
                    );
                    report.failed += 1;
                }
            }
        }
        // A provider that returned fewer vectors than inputs leaves the tail
        // pending; count them so the report reflects real progress.
        if vectors.len() < candidates.len() {
            report.failed += candidates.len() - vectors.len();
        }
        if report.reembedded > 0 {
            log::info!(
                "[memory::reembed] re-embedded {}/{} pending chunk(s)",
                report.reembedded,
                report.scanned
            );
        }
        report
    }

    /// Embed `texts`, retrying a transient provider failure with jittered
    /// exponential backoff (reusing `retry_after::backoff_ms_for_attempt`). An
    /// unrecoverable failure (per `classify_embed_error`) returns immediately —
    /// retrying it only burns the backoff budget against a condition a retry
    /// can never change.
    async fn embed_with_backoff(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut attempt = 0u32;
        loop {
            match self.embedder.embed(texts).await {
                Ok(vectors) => return Ok(vectors),
                Err(error) => {
                    if classify_embed_error(&error).is_unrecoverable() || attempt >= MAX_429_RETRIES
                    {
                        return Err(error);
                    }
                    let delay = jitter_ms(backoff_ms_for_attempt(attempt, None));
                    log::debug!(
                        "[memory::reembed] embed attempt {} failed, retrying in {delay}ms: {error:#}",
                        attempt + 1
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Stamp a freshly-computed vector onto its `vector_chunks` row, replacing a
    /// missing or wrong-dimension one. `dim` and `model_signature` are written
    /// together with the blob so a later scan and the recall dimension guard
    /// both read a consistent row.
    fn write_chunk_embedding(
        &self,
        candidate: &ReembedCandidate,
        vector: &[f32],
        signature: &str,
        now: f64,
    ) -> Result<(), String> {
        let bytes = Self::vec_to_bytes(vector);
        let dim = vector.len() as i64;
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE vector_chunks
                SET embedding = ?1, dim = ?2, model_signature = ?3, updated_at = ?4
              WHERE namespace = ?5 AND chunk_id = ?6",
            params![
                bytes,
                dim,
                signature,
                now,
                candidate.namespace,
                candidate.chunk_id
            ],
        )
        .map_err(|error| {
            format!(
                "write_chunk_embedding {}/{}: {error}",
                candidate.namespace, candidate.chunk_id
            )
        })?;
        Ok(())
    }
}

/// Spread a backoff delay across `[base/2, base]` so many instances retrying a
/// recovering backend do not resynchronise into a thundering herd. Uses the
/// clock's sub-millisecond noise as a dependency-free entropy source — retry
/// jitter needs spread, not cryptographic randomness.
fn jitter_ms(base_ms: u64) -> u64 {
    if base_ms == 0 {
        return 0;
    }
    let half = base_ms / 2;
    let noise = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::from(elapsed.subsec_nanos()))
        .unwrap_or(0);
    half + noise % (half + 1)
}

#[cfg(test)]
#[path = "reembed_tests.rs"]
mod tests;
