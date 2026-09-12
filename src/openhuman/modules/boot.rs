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
    super::memory::set_modules_policy(std::sync::Arc::new(config.clone()));
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
        if !should_eager_load(record, config) {
            log::debug!(
                "[modules] eager module '{}' skipped: the memory driver is not module-backed",
                record.id
            );
            continue;
        }
        // TinyMemory resolves its embedding provider while the library is
        // admitted, through callbacks this host serves on the module bus. The
        // lazy path installs them before loading; the eager path must too, or
        // the module comes up without an embedder and every memory write fails
        // from then on.
        if record.id == super::memory::MODULE_ID {
            if let Err(reason) =
                super::memory::install_host_callbacks(std::sync::Arc::new(config.clone())).await
            {
                log::warn!(
                    "[modules] eager module '{}' skipped: host callbacks are unavailable: {reason}",
                    record.id
                );
                continue;
            }
        }
        log::info!("[modules] eager module '{}' resolving at boot", record.id);
        if let Err(reason) = ops::ensure_loaded(config, record.id).await {
            log::warn!(
                "[modules] eager module '{}' did not load: {reason}",
                record.id
            );
            continue;
        }
        // The first retrieval after boot pays the Python server start, the
        // model load and the embedder's first connection — several seconds the
        // user's first question would otherwise wait on, or lose its memory
        // block to. Pay it now, off the request path (#6040).
        if record.id == super::memory::MODULE_ID {
            crate::openhuman::memory::auto_recall::warm::spawn_at_boot(std::sync::Arc::new(
                config.clone(),
            ));
        }
    }
}

/// Whether `record` should be loaded eagerly at boot, given `config`.
///
/// Every `LoadPolicy::Eager` record is eager unconditionally, except
/// TinyMemory: it is eager only when this host's memory subsystem actually
/// selected the module-backed driver. Eager-loading it for every
/// `modules.enabled` host — regardless of which memory driver is bound —
/// would mean a host on the (default) `Embedded` driver pays a startup
/// download and native `dlopen` for a module it never binds, breaking the
/// module driver's opt-in contract.
///
/// [`crate::openhuman::memory::binding::admit`] is the same pure,
/// side-effect-free check `memory::binding::build` itself uses to decide what
/// actually gets bound, so this can never disagree with the real binding.
fn should_eager_load(record: &super::types::ModuleRecord, config: &Config) -> bool {
    if record.id != super::memory::MODULE_ID {
        return true;
    }
    matches!(
        crate::openhuman::memory::binding::admit(&config.subsystems.memory),
        Ok((_, crate::core::subsystem::DriverClass::Module))
    )
}

#[cfg(test)]
#[path = "boot_tests.rs"]
mod tests;
