//! Two-lane explicit user preferences — namespaces, thresholds, read helpers.
//!
//! Preferences written by the `save_preference` tool live in one of two
//! namespaces depending on their relevance scope:
//!
//! - [`USER_PREF_GENERAL_NAMESPACE`] — always-on; injected into the system
//!   prompt at thread start (Lane A).
//! - [`USER_PREF_SITUATIONAL_NAMESPACE`] — topic-scoped; recalled per-turn by
//!   semantic similarity to the user's message (Lane B).
//!
//! Keeping the namespace constants and read helpers in one place lets the write
//! path, the system-prompt builder, and the per-turn recall path share one
//! definition.
//!
//! # Why this is host-side
//!
//! It used to live in the engine, and nothing about it was ever the engine's.
//! Which namespaces the two lanes use, how many standing preferences a prompt
//! may carry, how similar a hit must be before it counts as a contradiction —
//! those are product decisions about what to put in front of a model. A second
//! engine would have to reimplement them identically or the product would
//! change underneath it.
//!
//! The move cost no capability. The engine's `recall_relevant_by_vector` was
//! itself a *default* method over `query_namespace_hits`, filtering by the
//! vector component of the score breakdown; the contract exposes exactly that
//! query as [`MemoryRetrieval::recall_namespace_scored`], so the filter is
//! reproduced here verbatim rather than being asked for over the bus.
//!
//! # A driver without retrieval yields no preferences, not an error
//!
//! [`recall_by_vector`] returns empty when the bound driver does not advertise
//! [`Capability::Retrieval`](crate::openhuman::memory::api::capabilities::Capability::Retrieval).
//! That preserves the engine's behaviour — its default returned empty so
//! keyword-only backends opted out — and it is the right failure mode for both
//! callers: an absent Lane-B block and an absent contradiction check are
//! degradations, whereas an error would fail a chat turn or a preference write
//! over a capability the operator chose not to have.

use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::{MemoryCore as _, MemoryProvider};
use crate::openhuman::memory::guard::MemoryGuard;

/// Always-on preferences — injected into the system prompt every thread.
pub const USER_PREF_GENERAL_NAMESPACE: &str = "user_pref_general";

/// Topic-scoped preferences — recalled per query against the user's message.
pub const USER_PREF_SITUATIONAL_NAMESPACE: &str = "user_pref_situational";

/// Default cap on general preferences injected into the system prompt. Keeps
/// the always-on block bounded so it can't blow a small model's context window
/// (see the legacy `gpt-4` 8K overflow).
pub const STANDING_PREFS_LIMIT: usize = 10;

/// Top-K situational preferences to recall per turn (Lane B).
pub const SITUATIONAL_RECALL_LIMIT: usize = 5;

/// Minimum query↔preference vector similarity for a situational preference to
/// be injected. Below this the current message isn't considered relevant to the
/// preference, so nothing is injected (the "unrelated query → no block"
/// behaviour). Tunable against live data.
pub const SITUATIONAL_MIN_SIMILARITY: f64 = 0.35;

/// Minimum similarity for an existing preference to be flagged as a possible
/// contradiction of a newly-saved one. Higher than the Lane-B recall floor — we
/// only surface genuinely-close matches as contradiction candidates. Tunable.
pub const CONTRADICTION_SIMILARITY: f64 = 0.6;

/// Recall entries in `namespace` whose **vector** similarity alone clears
/// `min_vector_similarity`, as `(key, content)` pairs, most-relevant first —
/// over any [`MemoryProvider`], guarded or not.
///
/// Reproduces what the engine's `recall_relevant_by_vector` did: ask for the
/// scored hits, keep those whose `vector_similarity` component clears the
/// floor, and drop empty bodies. Filtering on that component rather than the
/// final score is the point — the combined score folds in keyword, graph and
/// freshness signals, so a lexically-similar but semantically-unrelated
/// preference would otherwise clear the bar.
///
/// This is the one body behind both doors into Lane B: the guard path
/// ([`recall_by_vector`], used by the contradiction check) and the session's
/// `Memory` handle (`DriverMemory::recall_relevant_by_vector`, #6041). A driver
/// without the retrieval family answers empty — the documented degradation —
/// while a driver that has it and fails answers `Err`, so each caller decides
/// what an error means for its turn.
pub(crate) async fn recall_by_vector_over(
    provider: &dyn MemoryProvider,
    namespace: &str,
    query: &str,
    limit: usize,
    min_vector_similarity: f64,
) -> Result<Vec<(String, String)>, MemoryError> {
    let Some(retrieval) = provider.as_retrieval() else {
        return Ok(Vec::new());
    };
    let started = std::time::Instant::now();
    let hits = retrieval
        .recall_namespace_scored(namespace, query, limit, None)
        .await?;
    // The floor is "tunable against live data", and this line is that data:
    // how close the best candidate came, whether or not it cleared. Keys and
    // scores only — never the preference text or the message.
    let top = hits
        .iter()
        .map(|h| h.score_breakdown.vector_similarity)
        .fold(f64::NEG_INFINITY, f64::max);
    let kept: Vec<(String, String)> = hits
        .into_iter()
        .filter(|h| h.score_breakdown.vector_similarity >= min_vector_similarity)
        .filter(|h| !h.content.trim().is_empty())
        .map(|h| (h.key, h.content))
        .collect();
    if top.is_finite() {
        log::info!(
            "[pref_recall] namespace={namespace} kept={} top_similarity={top:.3} floor={min_vector_similarity} elapsed_ms={}",
            kept.len(),
            started.elapsed().as_millis()
        );
    } else {
        log::debug!(
            "[pref_recall] namespace={namespace} no candidates elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }
    Ok(kept)
}

/// [`recall_by_vector_over`] through the guard, with a failed lookup read as
/// "nothing to inject": an absent Lane-B block or an absent contradiction check
/// is a degradation, whereas an error here would fail a chat turn or a
/// preference write.
async fn recall_by_vector(
    memory: &MemoryGuard,
    namespace: &str,
    query: &str,
    limit: usize,
    min_vector_similarity: f64,
) -> Vec<(String, String)> {
    match recall_by_vector_over(memory, namespace, query, limit, min_vector_similarity).await {
        Ok(hits) => hits,
        Err(err) => {
            // Degrade, but say so: a driver that has retrieval and fails is an
            // infrastructure signal, not the documented "no vectors" opt-out.
            log::warn!(
                "[pref_recall] vector recall failed namespace={namespace}; continuing without it: {err}"
            );
            Vec::new()
        }
    }
}

/// Load the latest-`limit` general preferences as plain-language strings,
/// newest-first (by `updated_at`). This is the Lane-A system-prompt block.
///
/// `list()` returns entries ordered newest-first but with `content` set to the
/// title (= topic key), so the body value is fetched via `get()`.
pub async fn load_general_preferences(memory: &MemoryGuard, limit: usize) -> Vec<String> {
    let entries = memory
        .list(Some(USER_PREF_GENERAL_NAMESPACE), None, None)
        .await
        .unwrap_or_default();

    // `limit` counts preferences the caller will actually see, so the blank
    // check comes first and the budget is spent only on kept values. Taking
    // `limit` entries up front instead would let a single blank newest entry
    // consume the whole budget and return nothing while a valid preference sat
    // one row behind it — the prompt block would quietly lose a standing
    // preference, with no error to notice.
    let mut out = Vec::new();
    for entry in entries {
        if out.len() >= limit {
            break;
        }
        if let Ok(Some(full)) = memory.get(USER_PREF_GENERAL_NAMESPACE, &entry.key).await {
            let value = full.content.trim();
            if !value.is_empty() {
                out.push(value.to_string());
            }
        }
    }
    out
}

/// [`load_general_preferences`] for a caller holding the session's own
/// storage handle rather than a guard.
///
/// The agent session binds its memory once at build time and threads the
/// `Arc<dyn Memory>` through the turn; resolving a guard ambiently there
/// would re-derive the workspace and could disagree with the handle the
/// session actually writes through. Same rule as the guard variant,
/// including the blank-skip: the budget is spent only on kept values, so a
/// blank newest entry cannot starve the prompt block of a real preference
/// one row behind it.
pub async fn load_general_preferences_on(
    memory: &std::sync::Arc<dyn crate::openhuman::memory::Memory>,
    limit: usize,
) -> Vec<String> {
    let entries = memory
        .list(Some(USER_PREF_GENERAL_NAMESPACE), None, None)
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for entry in entries {
        if out.len() >= limit {
            break;
        }
        if let Ok(Some(full)) = memory.get(USER_PREF_GENERAL_NAMESPACE, &entry.key).await {
            let value = full.content.trim();
            if !value.is_empty() {
                out.push(value.to_string());
            }
        }
    }
    out
}

/// [`recall_situational_preferences`] for the session's own storage handle.
///
/// `recall_relevant_by_vector` is a contract-trait method, so this stays
/// engine-neutral; a backend without vectors answers empty, which is the
/// documented degradation for Lane B — an absent block, never a failed turn.
/// The session handle is a `DriverMemory` on every module-backed install, and
/// that adapter answers through [`recall_by_vector_over`] (#6041) — it used to
/// sit on the trait's empty default, which made this lane a silent no-op.
pub async fn recall_situational_preferences_on(
    memory: &std::sync::Arc<dyn crate::openhuman::memory::Memory>,
    query: &str,
) -> Vec<String> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    // The vector recall embeds the message — a round trip to the embedder,
    // measured at 0.8–3 s on the desktop — and it runs on every turn. Most
    // users have never saved a topic-scoped preference, so ask the store the
    // cheap question first and only pay for the embed when there is something
    // to match. The question is a namespace *count*, not a listing: a user
    // with hundreds of preferences must not ship every row over the bus on
    // every turn just to learn that the namespace is non-empty. A failed
    // count falls through to the recall rather than silently disabling the
    // lane.
    let has_candidates = memory
        .namespace_summaries()
        .await
        .map(|summaries| {
            summaries
                .iter()
                .any(|s| s.namespace == USER_PREF_SITUATIONAL_NAMESPACE && s.count > 0)
        })
        .unwrap_or(true);
    if !has_candidates {
        log::debug!("[pref_recall] no situational preferences stored; skipping the vector recall");
        return Vec::new();
    }
    let recalled = match memory
        .recall_relevant_by_vector(
            USER_PREF_SITUATIONAL_NAMESPACE,
            query,
            SITUATIONAL_RECALL_LIMIT,
            SITUATIONAL_MIN_SIMILARITY,
        )
        .await
    {
        Ok(recalled) => recalled,
        Err(err) => {
            // An absent block, never a failed turn — but a failed lookup is
            // worth one line, or a broken driver reads as "no preferences".
            log::warn!(
                "[pref_recall] situational recall failed; continuing without a preference block: {err}"
            );
            Vec::new()
        }
    };
    recalled
        .into_iter()
        .map(|(_topic, value)| value)
        .filter(|value| !value.trim().is_empty())
        .collect()
}

/// Recall situational preferences semantically relevant to `query` (Lane B).
///
/// Returns only preferences whose vector similarity to the message clears
/// [`SITUATIONAL_MIN_SIMILARITY`], so an unrelated message yields an empty list
/// (and no injected block).
pub async fn recall_situational_preferences(memory: &MemoryGuard, query: &str) -> Vec<String> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    recall_by_vector(
        memory,
        USER_PREF_SITUATIONAL_NAMESPACE,
        query,
        SITUATIONAL_RECALL_LIMIT,
        SITUATIONAL_MIN_SIMILARITY,
    )
    .await
    .into_iter()
    .map(|(_topic, value)| value)
    .collect()
}

/// Find existing preferences (across both lanes) semantically close to `value`,
/// excluding `exclude_topic` (the just-saved one). Returns `(topic, value)`
/// pairs so the chat agent — which captured the preference in the first place —
/// can resolve a contradiction itself: overwrite the conflicting topic or remove
/// it. No separate model call; the conversation affirms it.
pub async fn recall_related_preferences(
    memory: &MemoryGuard,
    value: &str,
    exclude_topic: &str,
    limit: usize,
) -> Vec<(String, String)> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    // `limit` is a global cap across *both* lanes, not per-namespace — spend a
    // shared budget so the total surfaced for one contradiction check can never
    // exceed what the caller asked for.
    let mut remaining = limit;
    for ns in [USER_PREF_GENERAL_NAMESPACE, USER_PREF_SITUATIONAL_NAMESPACE] {
        if remaining == 0 {
            break;
        }
        for (topic, val) in
            recall_by_vector(memory, ns, value, remaining, CONTRADICTION_SIMILARITY).await
        {
            if topic != exclude_topic {
                out.push((topic, val));
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "preferences_tests.rs"]
mod tests;
