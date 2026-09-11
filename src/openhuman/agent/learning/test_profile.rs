//! An in-memory [`MemoryProfile`] for the learning tests.
//!
//! # Why this exists rather than `#[ignore]`
//!
//! The learning tests used to build a real `ProfileStore` over an in-memory
//! SQLite connection. That store moved behind the memory module, so those
//! constructions no longer compile — and the obvious response, parking the
//! tests on `OPENHUMAN_MODULE_PATH` like the tool tests, would have cost ~50
//! tests of coverage for no gain.
//!
//! It would also have been the wrong trade. Those tests are about *learning*
//! logic — stability scoring, class bucketing, prompt rendering, eviction — not
//! about storage. They only ever needed somewhere to put facets. So this
//! provides exactly that: a `HashMap` behind a mutex, implementing the same
//! contract the driver does.
//!
//! # Not `#[cfg(test)]`, deliberately
//!
//! Integration tests under `tests/` link the library compiled *without*
//! `cfg(test)`, so a test-gated helper is invisible to them — which is exactly
//! how `tests/learning_phase4_integration_test.rs` was left uncompilable once
//! before. `ProfileStore::for_tests` carries the same note and the same
//! `#[doc(hidden)]` treatment for the same reason.
//!
//! # It mimics the engine's ordering, because the tests depend on it
//!
//! `list_active` and `list_all` sort by stability descending, which is what the
//! engine's SQL does and what several assertions rely on. A fake that returned
//! insertion order would pass its own tests and quietly diverge from the thing
//! it stands in for.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::openhuman::agent::learning::cache::FacetCache;
use tinymemory_api::error::MemoryError;
use tinymemory_api::provider::{FacetType, MemoryProfile, ProfileFacet, UserState};

/// Facets held in memory, keyed by [`ProfileFacet::key`].
#[derive(Default)]
pub struct InMemoryProfile {
    facets: Mutex<HashMap<String, ProfileFacet>>,
    /// When set, `delete_facet` fails for this key. Lets a test drive the
    /// failure branch of a delete loop, which is the branch that decides
    /// whether a partial reset is reported as success.
    fail_delete_for: Mutex<Option<String>>,
}

impl InMemoryProfile {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Make `delete_facet` fail for `key`.
    pub fn fail_delete_for(&self, key: &str) {
        *self.fail_delete_for.lock() = Some(key.to_string());
    }

    /// Facets sorted the way the engine returns them: stability descending,
    /// then key ascending for a stable tie-break.
    fn sorted(&self, active_only: bool) -> Vec<ProfileFacet> {
        use tinymemory_api::provider::FacetState;
        let facets = self.facets.lock();
        let mut out: Vec<ProfileFacet> = facets
            .values()
            .filter(|f| !active_only || f.state == FacetState::Active)
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            b.stability
                .partial_cmp(&a.stability)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.key.cmp(&b.key))
        });
        out
    }
}

#[async_trait]
impl MemoryProfile for InMemoryProfile {
    async fn list_active_facets(&self) -> Result<Vec<ProfileFacet>, MemoryError> {
        Ok(self.sorted(true))
    }

    async fn list_all_facets(&self) -> Result<Vec<ProfileFacet>, MemoryError> {
        Ok(self.sorted(false))
    }

    async fn get_facet(&self, key: &str) -> Result<Option<ProfileFacet>, MemoryError> {
        Ok(self.facets.lock().get(key).cloned())
    }

    async fn facets_by_type(
        &self,
        facet_type: FacetType,
    ) -> Result<Vec<ProfileFacet>, MemoryError> {
        Ok(self
            .sorted(false)
            .into_iter()
            .filter(|f| f.facet_type == facet_type)
            .collect())
    }

    async fn upsert_facet(&self, facet: &ProfileFacet) -> Result<(), MemoryError> {
        self.facets.lock().insert(facet.key.clone(), facet.clone());
        Ok(())
    }

    async fn upsert_provider_facet(
        &self,
        facet_id: &str,
        facet_type: FacetType,
        key: &str,
        value: &str,
        confidence: f64,
        _segment_id: Option<&str>,
        observed_at: f64,
    ) -> Result<(), MemoryError> {
        let mut facets = self.facets.lock();
        let entry = facets
            .entry(key.to_string())
            .or_insert_with(|| ProfileFacet {
                facet_id: facet_id.to_string(),
                facet_type,
                key: key.to_string(),
                value: value.to_string(),
                confidence,
                evidence_count: 0,
                source_segment_ids: None,
                first_seen_at: observed_at,
                last_seen_at: observed_at,
                state: Default::default(),
                stability: 0.0,
                user_state: Default::default(),
                evidence_refs: Vec::new(),
                class: None,
                cue_families: None,
            });
        // Confidence-aware, like the engine: a weaker observation must not
        // overwrite a stronger one.
        if confidence >= entry.confidence {
            entry.value = value.to_string();
            entry.confidence = confidence;
        }
        entry.evidence_count += 1;
        entry.last_seen_at = observed_at;
        Ok(())
    }

    async fn set_facet_user_state(
        &self,
        key: &str,
        user_state: UserState,
    ) -> Result<bool, MemoryError> {
        match self.facets.lock().get_mut(key) {
            Some(facet) => {
                facet.user_state = user_state;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn delete_facet(&self, key: &str) -> Result<bool, MemoryError> {
        if self.fail_delete_for.lock().as_deref() == Some(key) {
            return Err(MemoryError::Other(anyhow::anyhow!(
                "simulated delete failure"
            )));
        }
        Ok(self.facets.lock().remove(key).is_some())
    }

    async fn delete_facet_by_id(&self, facet_id: &str) -> Result<bool, MemoryError> {
        let mut facets = self.facets.lock();
        let key = facets
            .values()
            .find(|f| f.facet_id == facet_id)
            .map(|f| f.key.clone());
        Ok(key.map(|k| facets.remove(&k)).is_some())
    }

    /// Matches the engine's predicate exactly:
    /// `stability < threshold AND user_state != 'pinned' AND state = 'dropped'`.
    ///
    /// Only **Dropped** rows are swept — an Active row below the threshold
    /// stays — and only **Pinned** is protected. A `Forgotten` facet is already
    /// Dropped and is meant to go.
    async fn drop_facets_below(&self, threshold: f64) -> Result<usize, MemoryError> {
        use tinymemory_api::provider::FacetState;
        let mut facets = self.facets.lock();
        let doomed: Vec<String> = facets
            .values()
            .filter(|f| {
                f.stability < threshold
                    && f.user_state != UserState::Pinned
                    && f.state == FacetState::Dropped
            })
            .map(|f| f.key.clone())
            .collect();
        let removed = doomed.len();
        for key in doomed {
            facets.remove(&key);
        }
        Ok(removed)
    }

    async fn workflow_identity_matches(&self, key_pattern: &str, canonical_value: &str) -> bool {
        // The engine takes a SQL `LIKE` pattern; the only shape the callers use
        // is a trailing `%`, so that is what this honours.
        let prefix = key_pattern.trim_end_matches('%');
        self.facets.lock().values().any(|f| {
            f.facet_type == FacetType::Workflow
                && f.key.starts_with(prefix)
                && f.value == canonical_value
        })
    }
}

/// A [`FacetCache`] over a fresh in-memory profile.
#[must_use]
pub fn in_memory_cache() -> FacetCache {
    FacetCache::for_tests(Arc::new(InMemoryProfile::new()))
}
