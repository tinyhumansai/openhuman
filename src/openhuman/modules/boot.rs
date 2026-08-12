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
use super::{host, ops, registry};
use crate::openhuman::config::Config;

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

    let runtime = match host::runtime().await {
        Ok(runtime) => runtime,
        Err(err) => {
            log::warn!("[modules] boot load skipped: the module bus could not start: {err}");
            return;
        }
    };

    // Search paths first: an artifact an operator has placed deliberately should
    // win over a download, and `ensure_loaded` below then finds it already
    // serving rather than fetching a second copy.
    for outcome in runtime.host().load_search_paths() {
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
        if let Err(reason) = ops::ensure_loaded(config, record.id).await {
            log::warn!(
                "[modules] eager module '{}' did not load: {reason}",
                record.id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::load_declared_modules;
    use crate::openhuman::config::Config;

    #[tokio::test]
    async fn boot_is_a_no_op_when_modules_are_disabled() {
        // Must not start a broker as a side effect of being switched off.
        let mut config = Config::default();
        config.modules.enabled = false;
        load_declared_modules(&config).await;
    }

    #[tokio::test]
    async fn boot_tolerates_an_empty_search_path() {
        // The ordinary case on a fresh machine: nothing installed, nothing eager,
        // and boot must complete rather than warn or fail.
        let mut config = Config::default();
        config.modules.enabled = true;
        config.modules.allow_download = false;
        load_declared_modules(&config).await;
    }
}
