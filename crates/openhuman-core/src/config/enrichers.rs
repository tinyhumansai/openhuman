//! Config enrichers: hooks the loader runs on every loaded [`Config`].
//!
//! An outside crate registers a function that fills the config's
//! `#[serde(skip)]` resolved fields (see
//! [`ResolvedEndpoints`](crate::config::schema::resolved_endpoints::ResolvedEndpoints))
//! from its own state. Enrichers run in registration order, after the file
//! (or the embedder's config) has been read and normalized, and must be
//! idempotent: the same config can be enriched more than once. They never
//! persist anything, because the fields they write cannot be serialized.
//!
//! With none registered a load returns exactly what it returned before.

use std::sync::Arc;

use super::Config;
use crate::core::slots::{Extensions, HookSlot};

/// A registered enricher.
pub type ConfigEnricher = Arc<dyn Fn(&mut Config) + Send + Sync>;

static ENRICHERS: HookSlot<ConfigEnricher> = HookSlot::new("config::enrichers");

/// Registers the enricher `id` for the whole process. Re-registering an id
/// replaces it and keeps its position.
pub fn register_enricher<F>(id: impl Into<String>, enricher: F)
where
    F: Fn(&mut Config) + Send + Sync + 'static,
{
    ENRICHERS.register(id, Arc::new(enricher));
}

/// Registers the enricher `id` for the context built from `extensions`.
pub fn register_enricher_in<F>(extensions: &Extensions, id: impl Into<String>, enricher: F)
where
    F: Fn(&mut Config) + Send + Sync + 'static,
{
    ENRICHERS.register_in(extensions, id, Arc::new(enricher));
}

/// Removes the process-wide enricher `id`.
pub fn unregister_enricher(id: &str) -> bool {
    ENRICHERS.unregister(id)
}

/// Runs every registered enricher over `config`, in registration order.
pub fn run_enrichers(config: &mut Config) {
    for (id, enricher) in ENRICHERS.entries() {
        tracing::trace!(enricher = %id, "[config] running enricher");
        enricher(config);
    }
}

/// The managed-host patterns for `config`: the ones already resolved onto it,
/// else the ones a registered enricher resolves.
///
/// Load-time migrations run before enrichment, so a freshly parsed config has
/// nothing resolved yet. Hosts are static (they do not depend on user
/// settings), so the registered enrichers are probed against a default config
/// instead. With no enricher registered the list is empty and nothing is
/// recognised as a managed endpoint.
pub(crate) fn managed_hosts_for(config: &Config) -> Vec<String> {
    if !config.resolved_endpoints.managed_hosts.is_empty() {
        return config.resolved_endpoints.managed_hosts.clone();
    }
    let mut probe = Config::default();
    run_enrichers(&mut probe);
    probe.resolved_endpoints.managed_hosts
}

/// Consumes `config`, enriched.
pub(crate) fn enriched(mut config: Config) -> Config {
    run_enrichers(&mut config);
    config
}

#[cfg(test)]
#[path = "enrichers_tests.rs"]
mod tests;
