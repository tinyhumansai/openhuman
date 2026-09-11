//! `FacetCache` — thin wrapper over `user_profile_facets` for Phase 3.
//!
//! Provides typed read/write access to the facet table with class-aware helpers.
//! The stability detector uses this to persist the result of each rebuild cycle.
//! Prompt sections use [`FacetCache::list_active`] to read the ambient cache.

use std::sync::Arc;

use crate::openhuman::agent::learning::candidate::FacetClass;
use crate::openhuman::memory::guard::MemoryGuard;
use tinymemory_api::provider::{MemoryProfile, MemoryProvider, ProfileFacet, UserState};

/// Thin wrapper around the profile facet store.
///
/// A learning-side newtype over the driver's
/// [`MemoryProfile`] family. This type exists because the class↔key vocabulary
/// below (`FacetClass`) is agent domain knowledge that must not move into the
/// memory contract; everything else forwards straight to the driver.
///
/// # Every method is async now, and that removed work rather than adding it
///
/// These used to be synchronous calls into an in-process SQLite handle, which
/// is why callers wrapped them in `spawn_blocking` — see
/// [`super::profile_md_renderer`]. With the store behind the module there is no
/// blocking I/O left in this process to move off the executor, so those hops
/// are gone and the calls are simply awaited.
pub struct FacetCache {
    source: Source,
}

/// Where a cache reads its facets from.
///
/// Production always takes [`Source::Guard`] — the bound driver, policy layer
/// included. [`Source::Direct`] exists for tests, which need somewhere to put
/// facets without standing up a driver; see
/// [`super::test_profile`] for why that is the right trade rather than parking
/// the learning tests on a module artifact.
enum Source {
    Guard(Arc<MemoryGuard>),
    Direct(Arc<dyn MemoryProfile>),
}

impl FacetCache {
    #[must_use]
    pub fn new(guard: Arc<MemoryGuard>) -> Self {
        Self {
            source: Source::Guard(guard),
        }
    }

    /// A cache over a caller-supplied profile family.
    ///
    /// Test-only: production must go through the guard so the policy layer is
    /// on the path.
    ///
    /// Not `#[cfg(test)]` — integration tests link the lib without it, and a
    /// gated constructor is invisible to them. `#[doc(hidden)]` keeps it off
    /// the public docs instead.
    #[doc(hidden)]
    #[must_use]
    pub fn for_tests(profile: Arc<dyn MemoryProfile>) -> Self {
        Self {
            source: Source::Direct(profile),
        }
    }

    /// The profile family, or a caller-facing error.
    fn profile(&self) -> anyhow::Result<&dyn MemoryProfile> {
        match &self.source {
            Source::Guard(guard) => guard.as_profile().ok_or_else(|| {
                anyhow::anyhow!("memory driver does not support the profile family")
            }),
            Source::Direct(profile) => Ok(profile.as_ref()),
        }
    }

    /// List all facets with `state = 'active'`, ordered by stability descending.
    pub async fn list_active(&self) -> anyhow::Result<Vec<ProfileFacet>> {
        Ok(self.profile()?.list_active_facets().await?)
    }

    /// List all facets (all states), ordered by stability descending.
    pub async fn list_all(&self) -> anyhow::Result<Vec<ProfileFacet>> {
        Ok(self.profile()?.list_all_facets().await?)
    }

    /// List active facets belonging to a specific class.
    ///
    /// Class is determined by the `key` prefix before the first `/`.
    pub async fn list_by_class(&self, class: FacetClass) -> anyhow::Result<Vec<ProfileFacet>> {
        let prefix = format!("{}/", class_prefix(class));
        let all = self.list_active().await?;
        Ok(all
            .into_iter()
            .filter(|f| f.key.starts_with(&prefix))
            .collect())
    }

    /// Fetch a single facet by its full key (e.g. `"style/verbosity"`).
    pub async fn get(&self, key: &str) -> anyhow::Result<Option<ProfileFacet>> {
        Ok(self.profile()?.get_facet(key).await?)
    }

    /// Upsert a fully-formed facet row (rebuild path).
    pub async fn upsert(&self, facet: &ProfileFacet) -> anyhow::Result<()> {
        Ok(self.profile()?.upsert_facet(facet).await?)
    }

    /// Override the `user_state` of a facet.
    ///
    /// Returns `Ok(true)` if a row was found and updated.
    pub async fn set_user_state(&self, key: &str, user_state: UserState) -> anyhow::Result<bool> {
        Ok(self
            .profile()?
            .set_facet_user_state(key, user_state)
            .await?)
    }

    /// Delete a facet by key. Returns `true` if a row was removed.
    pub async fn delete(&self, key: &str) -> anyhow::Result<bool> {
        Ok(self.profile()?.delete_facet(key).await?)
    }

    /// Delete all `Dropped`-state facets whose stability is below `threshold`.
    ///
    /// Pinned facets are never deleted. Returns the number of rows removed.
    pub async fn drop_below_threshold(&self, threshold: f64) -> anyhow::Result<usize> {
        Ok(self.profile()?.drop_facets_below(threshold).await?)
    }
}

// ── Class ↔ key utilities ─────────────────────────────────────────────────────

/// Extract the [`FacetClass`] from a full key string (e.g. `"style/verbosity"` → `Style`).
///
/// Returns `None` for keys that don't have a recognised class prefix.
pub fn class_from_key(key: &str) -> Option<FacetClass> {
    let prefix = key.split('/').next()?;
    match prefix {
        "style" => Some(FacetClass::Style),
        "identity" => Some(FacetClass::Identity),
        "tooling" => Some(FacetClass::Tooling),
        "veto" => Some(FacetClass::Veto),
        "goal" => Some(FacetClass::Goal),
        "channel" => Some(FacetClass::Channel),
        _ => None,
    }
}

/// Validate a caller-supplied class **name** against the facet taxonomy.
///
/// The learning RPC handlers and agent tools take `class` as a bare taxonomy
/// name (`"style"`, `"goal"`, …), not a full key. Reuse [`class_from_key`],
/// which matches on the first `/`-segment, so a bare name is validated as its
/// own prefix and an unknown name is rejected with a caller-facing error rather
/// than silently composing a key no facet can ever carry.
pub(crate) fn parse_facet_class_name(class: &str) -> Result<FacetClass, String> {
    // A taxonomy name never contains a `/`. Reject one up front so a value like
    // `"style/"` (which `class_from_key` would accept via its first segment,
    // then compose into a key no facet can carry) fails loudly instead of
    // reintroducing the silent-empty behaviour this fix removes.
    let invalid = || {
        format!(
            "invalid class `{class}` (expected one of style, identity, tooling, veto, goal, channel)"
        )
    };
    if class.contains('/') {
        return Err(invalid());
    }
    class_from_key(class).ok_or_else(invalid)
}

/// Delete every non-`Pinned` facet, returning `(deleted, pinned_preserved)`.
///
/// Shared by the `learning.reset_cache` RPC and the `learning_reset_cache`
/// agent tool so the two cannot drift — they answer the same user request and
/// previously carried two copies of this loop.
///
/// # Errors
///
/// Propagates a delete failure rather than counting it as "nothing to delete".
/// Swallowing it would answer a reset with success while leaving the facets in
/// place, which is the one outcome the caller cannot detect and the one that
/// matters: the next turn keeps reading material the user asked to forget.
/// `Ok(false)` from a delete is different and stays silent — the row was
/// already gone, which is the requested end state.
pub async fn reset_non_pinned(cache: &FacetCache) -> anyhow::Result<(usize, usize)> {
    let all = cache.list_all().await?;
    let pinned_preserved = all
        .iter()
        .filter(|f| f.user_state == UserState::Pinned)
        .count();

    let mut deleted = 0usize;
    for facet in &all {
        if facet.user_state == UserState::Pinned {
            continue;
        }
        if cache
            .delete(&facet.key)
            .await
            .map_err(|e| anyhow::anyhow!("delete failed after removing {deleted} facets: {e:#}"))?
        {
            deleted += 1;
        }
    }
    Ok((deleted, pinned_preserved))
}

/// Build a full key from a class and a suffix (e.g. `(Style, "verbosity")` → `"style/verbosity"`).
pub fn key_with_class(class: FacetClass, suffix: &str) -> String {
    format!("{}/{suffix}", class_prefix(class))
}

/// Return the canonical key prefix for a [`FacetClass`].
pub fn class_prefix(class: FacetClass) -> &'static str {
    match class {
        FacetClass::Style => "style",
        FacetClass::Identity => "identity",
        FacetClass::Tooling => "tooling",
        FacetClass::Veto => "veto",
        FacetClass::Goal => "goal",
        FacetClass::Channel => "channel",
    }
}

// ── Facet state enum re-export (convenience for callers of this module) ───────

pub use tinymemory_api::provider::{FacetState as CacheFacetState, UserState as CacheUserState};

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
