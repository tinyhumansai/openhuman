//! What happens to modules at startup.
//!
//! Two things, and deliberately not a third.
//!
//! Modules marked [`LoadPolicy::Eager`] are loaded, because their absence would
//! change what the core offers rather than merely delay it. Modules on the search
//! path are loaded, because that is how an operator or a developer puts an
//! artifact in front of this host without editing config — tinybus honours
//! `OPENHUMAN_MODULE_PATH` first, then the platform data directories.
//!
//! What does not happen is downloading every registry entry. A [`LoadPolicy::Lazy`]
//! module stays unloaded until something asks for it, so a user who never
//! produces a document never pays a download, a `dlopen`, or the resident cost of
//! a library that is never unloaded. Boot is also the worst moment to spend
//! network on something nobody has asked for yet.

use super::types::LoadPolicy;
use super::{ops, registry, runtime};
use crate::config::Config;

/// Load the modules that should be serving before the first request.
///
/// Never fails the boot: a module that cannot load leaves its feature
/// unavailable, and the feature says so at the point of use. Taking the core
/// down because an optional codec is missing would be a worse trade.
pub async fn load_declared_modules(config: &Config) {
    if !config.modules.enabled {
        log::debug!("[modules] boot load skipped: modules are disabled in configuration");
        return;
    }

    let runtime = match runtime::runtime().await {
        Ok(runtime) => runtime,
        Err(err) => {
            log::warn!("[modules] boot load skipped: the module bus could not start: {err}");
            return;
        }
    };

    // Search paths first: an artifact an operator has placed deliberately should
    // win over a download, and `ensure_loaded` below then finds it already
    // serving rather than fetching a second copy.
    for outcome in
        super::search_path::load_enabled(runtime.host(), &ops::disabled_lib_stems(config))
    {
        match outcome {
            Ok(info) => log::info!(
                "[modules] loaded '{}' {} from the module search path",
                info.name,
                info.manifest.module.version
            ),
            // Expected and not worth a warning: a search directory usually holds
            // nothing, and tinybus reports each refusal with a sanitised reason.
            Err(err) => log::debug!("[modules] search-path artifact not admitted: {err}"),
        }
    }

    for record in registry::ALL {
        if record.load != LoadPolicy::Eager {
            continue;
        }
        if let Some(component) = ops::disabled_component(config, record.id) {
            log::info!(
                "[modules] eager module '{}' skipped: component '{component}' is disabled",
                record.id
            );
            continue;
        }
        log::info!("[modules] eager module '{}' resolving at boot", record.id);
        if let Err(reason) = ops::ensure_loaded(config, record.id).await {
            log::warn!(
                "[modules] eager module '{}' did not load: {reason}",
                record.id
            );
        }
    }

    warm_connector_catalogs(config);
}

/// Read the connector module's curated catalogs into the host in the
/// background, so the synchronous lookups that gate an action (scope, approval,
/// flow checks) never run against an empty snapshot ahead of the first
/// connector call.
///
/// Off the boot path: the module may need loading (or downloading, when the
/// configuration allows it), and a slow or failed load must not hold the core
/// up. A failure is only logged; the next connector call primes it again. Not
/// started when the user turned Composio off or the connector component is
/// disabled.
fn warm_connector_catalogs(config: &Config) {
    if !should_warm_connector_catalogs(config) {
        log::debug!("[modules] connector catalog warm-up skipped: connectors are off");
        return;
    }
    // Unit tests share one process-wide module slot; a detached load started by
    // every boot test would race the tests that own it.
    if cfg!(test) {
        return;
    }
    let config = config.clone();
    tokio::spawn(async move {
        log::debug!("[modules] connector catalog warm-up starting");
        if let Err(reason) = ops::ensure_loaded(&config, super::connectors::MODULE_ID).await {
            log::debug!("[modules] connector catalog warm-up: module not loaded: {reason}");
            return;
        }
        super::connectors::prime_catalogs(&config).await;
    });
}

/// Whether the connectors are on: Composio is not switched off and the
/// connector component is not disabled.
fn should_warm_connector_catalogs(config: &Config) -> bool {
    use crate::config::schema::COMPOSIO_MODE_DISABLED;
    config.composio.mode.trim() != COMPOSIO_MODE_DISABLED
        && ops::disabled_component(config, super::connectors::MODULE_ID).is_none()
}

#[cfg(test)]
#[path = "boot_tests.rs"]
mod tests;
