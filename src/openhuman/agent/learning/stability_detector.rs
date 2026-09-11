//! Stability detector — Phase 3 of issue #566.
//!
//! Reads candidates from `candidate::global()`, aggregates them with existing
//! rows from `user_profile_facets`, scores each (class, key) pair by the
//! stability formula, resolves value conflicts, applies per-class budgets,
//! assigns lifecycle states, writes the result back to the table, and emits
//! [`DomainEvent::CacheRebuilt`].
//!
//! # Stability formula
//!
//! ```text
//! stability(class, key) = base × cue_mult × user_state_mult
//!
//! base = Σ(cue_family.weight() × exp(-Δt / half_life(class)) × ln(1 + evidence_count))
//! cue_mult  = 2.0 if any contributing evidence has CueFamily::Explicit, else 1.0
//! user_state_mult = ∞ if Pinned, 0 if Forgotten, 1 otherwise
//! ```
//!
//! # Thresholds
//!
//! | Symbol | Value | Meaning |
//! |--------|-------|---------|
//! | τ_promote | 1.5 | Enter as Active |
//! | τ_provisional | 0.7 | Enter as Provisional |
//! | τ_evict | 0.4 | Keep as Candidate |
//! | < τ_evict | — | Dropped |
//!
//! # Class budgets
//!
//! After state assignment, each class retains at most `class_budget(class)` Active
//! rows by stability. Excess Active rows are demoted to Provisional. A cross-class
//! overflow pool holds up to `BUDGET_OVERFLOW` extra Provisional rows.

use std::collections::{HashMap, HashSet};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::learning::cache::FacetCache;
use crate::openhuman::agent::learning::candidate::{
    self, CueFamily, FacetClass, LearningCandidate,
};
use tinymemory_api::provider::{FacetState, FacetType, ProfileFacet, UserState};

// ── Thresholds ────────────────────────────────────────────────────────────────

/// Stability threshold to enter / stay as Active.
pub const TAU_PROMOTE: f64 = 1.5;
/// Stability threshold to enter / stay as Provisional.
pub const TAU_PROVISIONAL: f64 = 0.7;
/// Stability threshold to retain as Candidate (below → Dropped).
pub const TAU_EVICT: f64 = 0.4;

// ── Half-lives (seconds) ──────────────────────────────────────────────────────

pub const HALF_LIFE_IDENTITY: f64 = 90.0 * 86400.0;
pub const HALF_LIFE_VETO: f64 = 60.0 * 86400.0;
pub const HALF_LIFE_TOOLING: f64 = 30.0 * 86400.0;
pub const HALF_LIFE_GOAL: f64 = 30.0 * 86400.0;
pub const HALF_LIFE_STYLE: f64 = 14.0 * 86400.0;
pub const HALF_LIFE_CHANNEL: f64 = 7.0 * 86400.0;

/// Half-life in seconds for the given class.
pub fn half_life(class: FacetClass) -> f64 {
    match class {
        FacetClass::Identity => HALF_LIFE_IDENTITY,
        FacetClass::Veto => HALF_LIFE_VETO,
        FacetClass::Tooling => HALF_LIFE_TOOLING,
        FacetClass::Goal => HALF_LIFE_GOAL,
        FacetClass::Style => HALF_LIFE_STYLE,
        FacetClass::Channel => HALF_LIFE_CHANNEL,
    }
}

// ── Class budgets ─────────────────────────────────────────────────────────────

pub const BUDGET_STYLE: usize = 4;
pub const BUDGET_IDENTITY: usize = 4;
pub const BUDGET_TOOLING: usize = 5;
pub const BUDGET_VETO: usize = 3;
pub const BUDGET_GOAL: usize = 3;
pub const BUDGET_CHANNEL: usize = 1;
/// Cross-class overflow pool for Provisional rows that didn't make a class budget.
pub const BUDGET_OVERFLOW: usize = 5;

/// Per-class top-N budget for Active rows.
pub fn class_budget(class: FacetClass) -> usize {
    match class {
        FacetClass::Style => BUDGET_STYLE,
        FacetClass::Identity => BUDGET_IDENTITY,
        FacetClass::Tooling => BUDGET_TOOLING,
        FacetClass::Veto => BUDGET_VETO,
        FacetClass::Goal => BUDGET_GOAL,
        FacetClass::Channel => BUDGET_CHANNEL,
    }
}

// ── Stability formula ─────────────────────────────────────────────────────────

/// Compute the stability score for a `(class, key)` aggregate.
///
/// # Arguments
///
/// * `cue_family` — dominant cue family of the evidence set
/// * `evidence_count` — total pieces of evidence contributing
/// * `last_reinforced_at` — Unix seconds of the most recent evidence
/// * `now` — current Unix seconds
/// * `class` — facet class (determines half-life)
/// * `has_explicit_evidence` — whether any evidence has `CueFamily::Explicit`
/// * `user_state` — user override (Pinned → ∞, Forgotten → 0)
pub fn stability(
    cue_family: CueFamily,
    evidence_count: u32,
    last_reinforced_at: f64,
    now: f64,
    class: FacetClass,
    has_explicit_evidence: bool,
    user_state: UserState,
) -> f64 {
    if matches!(user_state, UserState::Pinned) {
        return f64::INFINITY;
    }
    if matches!(user_state, UserState::Forgotten) {
        return 0.0;
    }

    let dt = (now - last_reinforced_at).max(0.0);
    let recency = (-dt / half_life(class)).exp();
    let base = cue_family.weight() * recency * (1.0 + evidence_count as f64).ln();
    let cue_mult = if has_explicit_evidence { 2.0 } else { 1.0 };
    base * cue_mult
}

// ── Rebuild outcome ───────────────────────────────────────────────────────────

/// Summary of a single rebuild cycle.
#[derive(Debug, Clone)]
pub struct RebuildOutcome {
    /// Facet rows newly created in this cycle (key not previously in the table).
    pub added: usize,
    /// Facet rows that were demoted to Dropped or deleted.
    pub evicted: usize,
    /// Facet rows carried over from the previous cycle without changes.
    pub kept: usize,
    /// Total rows in the cache after the rebuild.
    pub total_size: usize,
}

// ── StabilityDetector ─────────────────────────────────────────────────────────

/// The stability detector.
///
/// Owns a [`FacetCache`] and a reference to the global [`candidate::Buffer`].
/// Call [`StabilityDetector::rebuild`] to run one full cycle.
pub struct StabilityDetector {
    pub(crate) cache: FacetCache,
    pub(crate) buffer: &'static candidate::Buffer,
}

impl StabilityDetector {
    /// Create a new detector backed by the given cache.
    ///
    /// Uses [`candidate::global()`] as the source buffer.
    pub fn new(cache: FacetCache) -> Self {
        Self {
            cache,
            buffer: candidate::global(),
        }
    }

    /// Run one full rebuild cycle.
    ///
    /// 1. Drain the global candidate buffer.
    /// 2. Load all existing facets from the cache.
    /// 3. For each `(class, key)`, aggregate evidence + existing stability.
    /// 4. Choose the winning value via `argmax(stability)`.
    /// 5. Assign lifecycle state based on thresholds.
    /// 6. Apply per-class budgets (demote excess Active → Provisional).
    /// 7. Persist changes and delete Dropped rows.
    /// 8. Emit `DomainEvent::CacheRebuilt`.
    ///
    /// Async since the facet store moved behind the memory driver.
    pub async fn rebuild(&self, now: f64) -> anyhow::Result<RebuildOutcome> {
        tracing::debug!("[learning::stability] rebuild starting at t={now:.0}");

        // Step 1 — drain buffer.
        let candidates = self.buffer.drain();
        tracing::debug!(
            "[learning::stability] drained {} candidates from buffer",
            candidates.len()
        );

        // Step 2 — load existing facets.
        let existing_facets = self.cache.list_all().await?;
        let existing_by_key: HashMap<String, ProfileFacet> = existing_facets
            .into_iter()
            .map(|f| (f.key.clone(), f))
            .collect();

        // Step 3 — group candidates by (class, key).
        // For candidates whose key has no class prefix, we skip them (they're legacy rows).
        let mut groups: HashMap<(FacetClass, String), Vec<LearningCandidate>> = HashMap::new();

        for cand in candidates {
            let full_key = format!("{}/{}", class_prefix(cand.class), cand.key);
            groups.entry((cand.class, full_key)).or_default().push(cand);
        }

        // Also process existing facets that have a known class prefix (so they decay).
        for key in existing_by_key.keys() {
            if let Some(class) = crate::openhuman::agent::learning::cache::class_from_key(key) {
                groups.entry((class, key.to_string())).or_default(); // ensure the group exists even if no new candidates
            }
        }

        // Step 4-6: compute new state for each (class, key).
        // We accumulate the final facets per class so we can apply budgets.
        let mut by_class: HashMap<FacetClass, Vec<ComputedFacet>> = HashMap::new();

        for ((class, full_key), cands) in &groups {
            let existing = existing_by_key.get(full_key);

            // Respect Forgotten user_state: block re-promotion.
            let user_state = existing.map(|f| f.user_state).unwrap_or(UserState::Auto);

            // Step 4: for each distinct value, compute a candidate score.
            let winning_value = select_winning_value(cands, existing, now, *class);

            // Step 5: compute stability of the winning (class, key) aggregate.
            let (agg_score, has_explicit) = aggregate_stability(cands, existing, now, *class);
            let final_stability = stability(
                dominant_cue(cands, existing),
                total_evidence_count(cands, existing),
                most_recent_reinforcement(cands, existing, now, *class),
                now,
                *class,
                has_explicit,
                user_state,
            );

            tracing::trace!(
                "[learning::stability] {full_key}: value={:?} agg={agg_score:.3} final={final_stability:.3}",
                winning_value.as_deref().unwrap_or("(none)"),
            );

            // Step 7: state assignment.
            let state = state_from_stability(final_stability, user_state);

            let value = match winning_value.or_else(|| existing.map(|f| f.value.clone())) {
                Some(v) => v,
                None => {
                    // No candidates and no existing row — shouldn't happen but skip.
                    continue;
                }
            };

            let existing_count = existing.map(|f| f.evidence_count).unwrap_or(0);
            let new_evidence_count = (existing_count + cands.len() as i32).max(1);

            let facet_id = existing
                .map(|f| f.facet_id.clone())
                .unwrap_or_else(|| format!("learn-{}", full_key.replace('/', "-")));

            let first_seen = existing.map(|f| f.first_seen_at).unwrap_or(now);

            // Collect evidence refs from new candidates.
            let new_refs: Vec<crate::openhuman::agent::learning::candidate::EvidenceRef> =
                cands.iter().map(|c| c.evidence.clone()).collect();

            let existing_refs = existing
                .map(|f| evidence_from_contract(&f.evidence_refs))
                .unwrap_or_default();
            let all_refs = merge_evidence_refs(&existing_refs, new_refs);

            // Build cue-families counts from this cycle's candidates.
            let mut cue_counts: HashMap<String, u32> = HashMap::new();
            for c in cands {
                *cue_counts
                    .entry(format!("{:?}", c.cue_family).to_lowercase())
                    .or_insert(0) += 1;
            }
            // Merge with existing cue counts if present.
            if let Some(existing_cues) = existing.and_then(|f| f.cue_families.as_ref()) {
                for (k, v) in existing_cues {
                    *cue_counts.entry(k.clone()).or_insert(0) += v;
                }
            }

            let computed = ComputedFacet {
                is_new: existing.is_none(),
                facet: ProfileFacet {
                    facet_id,
                    facet_type: FacetType::Preference,
                    key: full_key.clone(),
                    value,
                    confidence: agg_score.clamp(0.0, 1.0),
                    evidence_count: new_evidence_count,
                    source_segment_ids: existing.and_then(|f| f.source_segment_ids.clone()),
                    first_seen_at: first_seen,
                    last_seen_at: now,
                    state,
                    stability: final_stability,
                    user_state,
                    evidence_refs: evidence_to_contract(&all_refs),
                    // Class derived from the key prefix (always set for learning rows).
                    class: Some(class_prefix(*class).to_string()),
                    cue_families: if cue_counts.is_empty() {
                        None
                    } else {
                        Some(cue_counts)
                    },
                },
            };

            by_class.entry(*class).or_default().push(computed);
        }

        // Step 6 (budget): per-class enforce top-N Active.
        let mut overflow_provisional: Vec<ComputedFacet> = Vec::new();
        let mut all_final: Vec<ComputedFacet> = Vec::new();

        for (class, mut facets) in by_class {
            // Sort Active rows by stability desc.
            facets.sort_by(|a, b| {
                b.facet
                    .stability
                    .partial_cmp(&a.facet.stability)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let budget = class_budget(class);
            let mut active_count = 0usize;

            for mut cf in facets {
                if cf.facet.state == FacetState::Active {
                    if active_count < budget {
                        active_count += 1;
                    } else {
                        // Demote excess Active → Provisional and send to overflow pool.
                        cf.facet.state = FacetState::Provisional;
                        overflow_provisional.push(cf);
                        continue;
                    }
                }
                all_final.push(cf);
            }
        }

        // Apply overflow budget: keep top BUDGET_OVERFLOW Provisional rows.
        overflow_provisional.sort_by(|a, b| {
            b.facet
                .stability
                .partial_cmp(&a.facet.stability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for cf in overflow_provisional.into_iter().take(BUDGET_OVERFLOW) {
            all_final.push(cf);
        }

        // Step 7 — persist and compute outcome.
        let mut added = 0usize;
        let mut kept = 0usize;
        let mut evicted = 0usize;

        for cf in &all_final {
            if cf.facet.state == FacetState::Dropped {
                evicted += 1;
            } else if cf.is_new {
                added += 1;
            } else {
                kept += 1;
            }
            self.cache.upsert(&cf.facet).await?;
        }

        // (Existing keys not in the rebuild output are legacy/non-class rows — skip.)

        // Clean up Dropped rows from the table.
        let cleaned = self.cache.drop_below_threshold(TAU_EVICT).await?;
        if cleaned > 0 {
            tracing::debug!(
                "[learning::stability] cleaned {cleaned} rows below threshold from table"
            );
        }

        let active_rows = self.cache.list_active().await?;
        let total_size = active_rows.len();

        tracing::info!(
            "[learning::stability] rebuild added={added} evicted={evicted} kept={kept} total={total_size}"
        );

        // Step 8 — publish CacheRebuilt event.
        BUS.publish(DomainEvent::CacheRebuilt {
            added,
            evicted,
            kept,
            total_size,
            rebuilt_at: now,
        });

        Ok(RebuildOutcome {
            added,
            evicted,
            kept,
            total_size,
        })
    }
}

// ── Rebuild internals ─────────────────────────────────────────────────────────

struct ComputedFacet {
    is_new: bool,
    facet: ProfileFacet,
}

/// Choose the winning value for a `(class, key)` group via `argmax(stability)`.
///
/// Returns the value with the highest combined evidence weight. Falls back to the
/// existing row's value if no candidates are present.
fn select_winning_value(
    cands: &[LearningCandidate],
    existing: Option<&ProfileFacet>,
    now: f64,
    class: FacetClass,
) -> Option<String> {
    if cands.is_empty() {
        return existing.map(|f| f.value.clone());
    }

    // Score each distinct value.
    let mut value_scores: HashMap<&str, f64> = HashMap::new();
    for c in cands {
        let dt = (now - c.observed_at).max(0.0);
        let recency = (-dt / half_life(class)).exp();
        let score = c.cue_family.weight() * recency * c.initial_confidence;
        *value_scores.entry(c.value.as_str()).or_default() += score;
    }

    // If existing row matches a candidate value, add its weight too.
    if let Some(existing) = existing {
        let dt = (now - existing.last_seen_at).max(0.0);
        let recency = (-dt / half_life(class)).exp();
        let existing_score = recency * existing.confidence;
        *value_scores.entry(existing.value.as_str()).or_default() += existing_score;
    }

    value_scores
        .into_iter()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(v, _)| v.to_string())
}

/// Aggregate stability contribution from all candidates (not per-value).
/// Returns (aggregate_score, has_explicit_evidence).
fn aggregate_stability(
    cands: &[LearningCandidate],
    existing: Option<&ProfileFacet>,
    now: f64,
    class: FacetClass,
) -> (f64, bool) {
    let mut score = 0.0f64;
    let mut has_explicit = false;

    for c in cands {
        let dt = (now - c.observed_at).max(0.0);
        let recency = (-dt / half_life(class)).exp();
        score += c.cue_family.weight() * recency;
        if matches!(c.cue_family, CueFamily::Explicit) {
            has_explicit = true;
        }
    }

    if let Some(existing) = existing {
        let dt = (now - existing.last_seen_at).max(0.0);
        let recency = (-dt / half_life(class)).exp();
        score += existing.confidence * recency;
    }

    (score, has_explicit)
}

/// Determine the dominant cue family (highest weight).
fn dominant_cue(cands: &[LearningCandidate], _existing: Option<&ProfileFacet>) -> CueFamily {
    cands
        .iter()
        .max_by(|a, b| {
            a.cue_family
                .weight()
                .partial_cmp(&b.cue_family.weight())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|c| c.cue_family)
        .unwrap_or(CueFamily::Behavioral)
}

/// Convert the learning domain's `EvidenceRef` to the memory contract's.
///
/// # Why a conversion and not one type
///
/// They are the *same shape* — `memory/api/host/evidence.rs` and
/// `tinymemory-api`'s copy are byte-identical, and this round-trips through
/// serde precisely because of that. They are nominally distinct only because
/// the learning candidate types still live in `tinymemory_core`, so
/// `candidate::EvidenceRef` resolves to the crate's copy while
/// `ProfileFacet::evidence_refs` uses the host's.
///
/// This bridge disappears when `learning_candidate` comes home — it is agent
/// domain knowledge, not engine storage, and belongs host-side with the rest of
/// the learning subsystem. Tracked as stage 4 in
/// `docs/specs/2026-08-13-memory-module-port.md`.
fn evidence_to_contract(refs: &[candidate::EvidenceRef]) -> Vec<tinymemory_api::host::EvidenceRef> {
    refs.iter()
        .filter_map(|r| {
            serde_json::to_value(r)
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
        })
        .collect()
}

/// The inverse of [`evidence_to_contract`].
fn evidence_from_contract(
    refs: &[tinymemory_api::host::EvidenceRef],
) -> Vec<candidate::EvidenceRef> {
    refs.iter()
        .filter_map(|r| {
            serde_json::to_value(r)
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
        })
        .collect()
}

/// Merge the existing row's evidence refs with this cycle's new refs,
/// deduplicating while preserving first-seen order.
///
/// `Vec::dedup_by` only collapses *consecutive* equal elements, so a ref that
/// recurs non-adjacently — present in the existing row and re-emitted by a new
/// candidate, or repeated within one cycle — would slip through and accumulate
/// without bound across rebuilds. `EvidenceRef: Eq + Hash`, so tracking seen
/// refs in a set removes every duplicate exactly and cheaply.
fn merge_evidence_refs(
    existing_refs: &[candidate::EvidenceRef],
    new_refs: Vec<candidate::EvidenceRef>,
) -> Vec<candidate::EvidenceRef> {
    let mut seen: HashSet<candidate::EvidenceRef> = HashSet::new();
    existing_refs
        .iter()
        .cloned()
        .chain(new_refs)
        .filter(|r| seen.insert(r.clone()))
        .collect()
}

/// Total evidence count from candidates + existing row.
fn total_evidence_count(cands: &[LearningCandidate], existing: Option<&ProfileFacet>) -> u32 {
    let from_existing = existing.map(|f| f.evidence_count as u32).unwrap_or(0);
    from_existing + cands.len() as u32
}

/// The most recent observation timestamp across candidates and the existing row.
///
/// The result is floored at `now - half_life(class)` so a facet's recency decay
/// in [`stability`] bottoms out at one (class-specific) half-life. The floor
/// must use the facet's own `class`: every other per-group computation in
/// `rebuild` is class-scoped, and the half-lives span 7d (Channel) to 90d
/// (Identity), so a hardcoded class would over-retain longer-lived facets and
/// evict shorter-lived ones too early.
fn most_recent_reinforcement(
    cands: &[LearningCandidate],
    existing: Option<&ProfileFacet>,
    now: f64,
    class: FacetClass,
) -> f64 {
    let newest_cand = cands
        .iter()
        .map(|c| c.observed_at)
        .fold(f64::NEG_INFINITY, f64::max);
    let existing_ts = existing
        .map(|f| f.last_seen_at)
        .unwrap_or(f64::NEG_INFINITY);
    newest_cand.max(existing_ts).max(now - half_life(class))
}

/// Map a stability score + user_state to a lifecycle state.
fn state_from_stability(score: f64, user_state: UserState) -> FacetState {
    // Pinned → always Active; Forgotten → always Dropped.
    if matches!(user_state, UserState::Pinned) {
        return FacetState::Active;
    }
    if matches!(user_state, UserState::Forgotten) {
        return FacetState::Dropped;
    }

    if score.is_infinite() || score >= TAU_PROMOTE {
        FacetState::Active
    } else if score >= TAU_PROVISIONAL {
        FacetState::Provisional
    } else if score >= TAU_EVICT {
        FacetState::Candidate
    } else {
        FacetState::Dropped
    }
}

/// Canonical key prefix string for a class (used when grouping candidates).
fn class_prefix(class: FacetClass) -> &'static str {
    crate::openhuman::agent::learning::cache::class_prefix(class)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "stability_detector_tests.rs"]
mod tests;
