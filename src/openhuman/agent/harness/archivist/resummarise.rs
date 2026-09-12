//! Recovering segments a failed recap left unsummarised (#6186).
//!
//! #6156 stopped the archivist persisting a heuristic bookend when the
//! summariser fails, which leaves the segment `status='closed'` with a `NULL`
//! summary. That is deliberately the `needs_resummary` marker — it is what
//! `SegmentsPendingSummary` selects on — but a marker nothing reads is only a
//! tidier way to lose the summary. This is the pass that reads it.
//!
//! **What triggers it.** Not a timer. The pass runs immediately after a recap
//! that *succeeded*, because that success is the only first-hand evidence the
//! app ever gets that the summariser is answering right now. A scheduler would
//! have to guess, and would spend its budget hammering a provider that is still
//! down; this cannot run at all while recaps are failing, which is exactly the
//! behaviour a backoff would be approximating.
//!
//! **Why it is bounded twice.** `SegmentsPendingSummary` orders oldest-first,
//! so a segment nothing can ever summarise sits at the head of the queue
//! forever. Draining `limit` rows every time would re-attempt that same head
//! on every segment close and never reach the rest — the head-of-queue trap.
//! So there is a per-process attempt ledger as well as a batch cap: a segment
//! that has already been tried [`MAX_ATTEMPTS_PER_SEGMENT`] times in this
//! process is skipped, and the pass moves on to segments behind it.

use std::collections::HashMap;
use std::sync::Mutex;

use super::lifecycle::recap_is_usable;
use super::ArchivistHook;
use crate::openhuman::memory::api::provider::episodic::EpisodicTurn;

/// Segments re-summarised in one pass.
///
/// Small on purpose. The pass runs after *every* successful segment close, so
/// the queue drains across many passes rather than in one long stall; a large
/// batch would put a run of inference calls behind a single close, and
/// `flush_open_segment` is awaited at session wind-down.
const RESUMMARISE_BATCH: usize = 3;

/// How far down the pending queue one pass will look to find those segments.
///
/// The batch cap alone is not enough, and this is the correction to the first
/// version of this pass. `segments_pending_summary` orders oldest-first and
/// takes a `limit`, so asking for exactly [`RESUMMARISE_BATCH`] returns the
/// three oldest rows *before* the attempt ledger filters them. Once those three
/// exhaust their attempts they still occupy every result, and a fourth pending
/// segment can never be reached until the process restarts — the head-of-queue
/// trap, moved one layer out rather than solved.
///
/// So the pass reads a window and stops after [`RESUMMARISE_BATCH`] **eligible**
/// segments. Bounded rather than unbounded because the point is to make
/// progress past a stuck head, not to walk an arbitrarily long backlog inside
/// one segment close; a queue deeper than this drains across passes, which is
/// what the batch cap is for.
const RESUMMARISE_SCAN: u32 = 25;

/// How many times one segment may be attempted before this process gives up on
/// it.
///
/// Bounds the head-of-queue trap described in the module docs. Deliberately
/// per-process rather than persisted: a restart usually means new config or a
/// new build, which is the one thing likely to change the answer, so a fresh
/// process earns each segment one more look.
const MAX_ATTEMPTS_PER_SEGMENT: u32 = 2;

/// Attempts spent per segment id, for this process.
static ATTEMPTS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

/// Record an attempt against `segment_id` and report whether it should be
/// skipped as already exhausted.
fn exhausted(segment_id: &str) -> bool {
    let mut guard = match ATTEMPTS.lock() {
        Ok(guard) => guard,
        // A poisoned ledger means some other pass panicked mid-update. The
        // ledger is an optimisation, not a correctness invariant, so the honest
        // response is to let this segment through rather than to stop
        // recovering summaries for the rest of the process's life.
        Err(poisoned) => poisoned.into_inner(),
    };
    let ledger = guard.get_or_insert_with(HashMap::new);
    let seen = ledger.entry(segment_id.to_string()).or_insert(0);
    if *seen >= MAX_ATTEMPTS_PER_SEGMENT {
        return true;
    }
    *seen += 1;
    false
}

impl ArchivistHook {
    /// Re-summarise up to [`RESUMMARISE_BATCH`] segments whose recap failed
    /// earlier.
    ///
    /// Uses the same [`ArchivistHook::summarize_entries`] the finalize path
    /// uses — not a second summariser. That is the whole reason the pass lives
    /// here rather than inside the engine: a differently-prompted summary
    /// sitting beside the originals would be indistinguishable from them and
    /// impossible to audit later.
    ///
    /// Writes only on a usable recap, by the same [`recap_is_usable`] rule as
    /// finalize. A segment whose recap fails again is left exactly as it was,
    /// still carrying the marker.
    ///
    /// Never returns `Err`: every failure is logged and the pass moves to the
    /// next segment. It is opportunistic work, and a failure here must not
    /// affect the close that triggered it.
    pub(super) async fn resummarise_pending(&self, now: f64) {
        let Some(episodic) = self.episodic() else {
            return;
        };

        let pending = match episodic.segments_pending_summary(RESUMMARISE_SCAN).await {
            Ok(pending) => pending,
            Err(e) => {
                tracing::debug!("[archivist] resummarise: cannot read the pending queue: {e}");
                return;
            }
        };
        if pending.is_empty() {
            return;
        }

        tracing::debug!(
            "[archivist] resummarise: {} segment(s) pending",
            pending.len()
        );

        let mut attempted = 0_usize;
        for segment in pending {
            if attempted >= RESUMMARISE_BATCH {
                break;
            }
            if exhausted(&segment.segment_id) {
                tracing::debug!(
                    "[archivist] resummarise: segment={} already attempted {} times in this \
                     process — skipping so the queue behind it can drain",
                    segment.segment_id,
                    MAX_ATTEMPTS_PER_SEGMENT
                );
                continue;
            }

            // Read per segment rather than once for the batch: turns are
            // addressed by session, and two pending segments are usually from
            // different sessions.
            //
            // `read_session_entries` + `is_in_segment` is the same pair the
            // finalize path uses, and reusing it is not incidental — a turn
            // belongs to a segment by stable per-session sequence or row id,
            // not by timestamp, because the md store rounds to milliseconds and
            // can sort a fast turn just before its own segment's start.
            // Counted here rather than at the top of the loop: a skipped
            // exhausted segment must not consume the batch, which is the whole
            // point of scanning past it.
            attempted += 1;
            let entries = self.read_session_entries(&segment.session_id).await;
            let segment_entries: Vec<&EpisodicTurn> = entries
                .iter()
                .filter(|record| record.is_in_segment(&segment))
                .map(|record| &record.turn)
                .collect();
            if segment_entries.is_empty() {
                // Nothing to fold. Not an error and not worth a warning: the
                // segment is a real row whose turns have since been pruned, and
                // there is no recap that could be produced for it.
                tracing::debug!(
                    "[archivist] resummarise: segment={} has no turns left — nothing to fold",
                    segment.segment_id
                );
                continue;
            }

            let (summary, from_llm) = self
                .summarize_entries(&segment_entries, &segment.segment_id, segment.turn_count)
                .await;
            if !recap_is_usable(from_llm, &summary) {
                tracing::debug!(
                    "[archivist] resummarise: segment={} still has no LLM recap — left \
                     unsummarised",
                    segment.segment_id
                );
                continue;
            }

            match episodic
                .set_segment_summary(&segment.segment_id, &summary, now)
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        "[archivist] resummarise: recovered segment={} ({} turns, {} chars)",
                        segment.segment_id,
                        segment.turn_count,
                        summary.len()
                    );
                    // Embedded here for the same reason finalize embeds: the
                    // summary is only in the index if something puts it there,
                    // and a recovered segment that is durable but unindexed is
                    // a second, quieter version of the same gap.
                    self.embed_segment_recap(&segment.segment_id, &summary, now)
                        .await;
                }
                Err(e) => {
                    tracing::warn!(
                        "[archivist] resummarise: failed to persist the recovered recap for \
                         segment={}: {e}",
                        segment.segment_id
                    );
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "resummarise_tests.rs"]
mod tests;
