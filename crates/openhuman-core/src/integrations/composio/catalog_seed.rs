//! Test-only stand-in for the module's catalog and probe caches.
//!
//! The real caches live inside the loaded `tinyconnectors` module, which unit
//! tests cannot reach, so tests seed contracts here and
//! [`super::fetch_live_toolkit_catalog`] answers from them first. A seeded
//! probe is overlaid the way the module overlays a cached one.

use super::{ProbedOutputSample, ToolContract};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static CATALOGS: OnceLock<Mutex<HashMap<String, Vec<ToolContract>>>> = OnceLock::new();
static PROBES: OnceLock<Mutex<HashMap<String, ProbedOutputSample>>> = OnceLock::new();

fn key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

pub(crate) fn seed_live_catalog_cache(toolkit: &str, contracts: Vec<ToolContract>) {
    CATALOGS
        .get_or_init(Default::default)
        .lock()
        .expect("catalog seed poisoned")
        .insert(key(toolkit), contracts);
}

/// An expired module entry means a refetch; with no module in unit tests that
/// is "unknown", so expiring removes the seed.
pub(crate) fn seed_live_catalog_cache_expired(toolkit: &str, _contracts: Vec<ToolContract>) {
    CATALOGS
        .get_or_init(Default::default)
        .lock()
        .expect("catalog seed poisoned")
        .remove(&key(toolkit));
}

pub(crate) fn seed_probe_cache(slug: &str, sample: ProbedOutputSample) {
    PROBES
        .get_or_init(Default::default)
        .lock()
        .expect("probe seed poisoned")
        .insert(key(slug), sample);
}

pub(super) fn seeded_catalog(toolkit: &str) -> Option<Vec<ToolContract>> {
    let mut contracts = CATALOGS
        .get_or_init(Default::default)
        .lock()
        .expect("catalog seed poisoned")
        .get(&key(toolkit))
        .cloned()?;
    let probes = PROBES.get_or_init(Default::default).lock().ok()?;
    for contract in &mut contracts {
        if let Some(probe) = probes.get(&key(&contract.slug)) {
            contract.primary_array_path = probe.primary_array_path.clone();
            if !probe.output_fields.is_empty() {
                contract.output_fields = probe.output_fields.clone();
            }
        }
    }
    Some(contracts)
}
