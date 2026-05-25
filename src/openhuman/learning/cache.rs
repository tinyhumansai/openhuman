//! `FacetCache` — thin wrapper over `user_profile_facets` for Phase 3.
//!
//! Provides typed read/write access to the facet table with class-aware helpers.
//! The stability detector uses this to persist the result of each rebuild cycle.
//! Prompt sections use [`FacetCache::list_active`] to read the ambient cache.

use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

use crate::openhuman::learning::candidate::FacetClass;
use crate::openhuman::memory_store::profile::{self, ProfileFacet, UserState};

/// Thin wrapper around the `user_profile` table.
///
/// All methods delegate to the standalone helpers in
/// `memory::store::unified::profile`. This type exists so callers
/// (stability detector, prompt sections, RPCs) share a single typed
/// entry-point that can be constructed from any `Arc<Mutex<Connection>>`.
pub struct FacetCache {
    conn: Arc<Mutex<Connection>>,
}

impl FacetCache {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// List all facets with `state = 'active'`, ordered by stability descending.
    pub fn list_active(&self) -> anyhow::Result<Vec<ProfileFacet>> {
        profile::profile_select_active(&self.conn)
    }

    /// List all facets (all states), ordered by stability descending.
    pub fn list_all(&self) -> anyhow::Result<Vec<ProfileFacet>> {
        profile::profile_select_all(&self.conn)
    }

    /// List active facets belonging to a specific class.
    ///
    /// Class is determined by the `key` prefix before the first `/`.
    pub fn list_by_class(&self, class: FacetClass) -> anyhow::Result<Vec<ProfileFacet>> {
        let prefix = format!("{}/", class_prefix(class));
        let all = self.list_active()?;
        Ok(all
            .into_iter()
            .filter(|f| f.key.starts_with(&prefix))
            .collect())
    }

    /// Fetch a single facet by its full key (e.g. `"style/verbosity"`).
    pub fn get(&self, key: &str) -> anyhow::Result<Option<ProfileFacet>> {
        profile::profile_get_by_key(&self.conn, key)
    }

    /// Upsert a fully-formed facet row (rebuild path).
    pub fn upsert(&self, facet: &ProfileFacet) -> anyhow::Result<()> {
        profile::profile_upsert_full(&self.conn, facet)
    }

    /// Override the `user_state` of a facet.
    ///
    /// Returns `Ok(true)` if a row was found and updated.
    pub fn set_user_state(&self, key: &str, user_state: UserState) -> anyhow::Result<bool> {
        profile::profile_set_user_state(&self.conn, key, user_state)
    }

    /// Delete a facet by key. Returns `true` if a row was removed.
    pub fn delete(&self, key: &str) -> anyhow::Result<bool> {
        profile::profile_delete_by_key(&self.conn, key)
    }

    /// Delete all `Dropped`-state facets whose stability is below `threshold`.
    ///
    /// Pinned facets are never deleted. Returns the number of rows removed.
    pub fn drop_below_threshold(&self, threshold: f64) -> anyhow::Result<usize> {
        profile::profile_delete_below_threshold(&self.conn, threshold)
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

pub use crate::openhuman::memory_store::profile::{
    FacetState as CacheFacetState, UserState as CacheUserState,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
