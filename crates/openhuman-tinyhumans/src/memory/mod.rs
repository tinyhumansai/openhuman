//! The hosted `tinyhumans` memory engine and its free-period check.
//!
//! [`register`] hands the core's memory engine registry
//! (`openhuman_embed::__host::memory_engines`) the engine id `tinyhumans`:
//!
//! - [`engine`] binds TinyMemory's CortexDB engine to the TinyHumans backend
//!   (endpoint, attribution headers, per-request bearer), below the layout
//!   root the core passes in the [`EngineRequest`] (`memory::scope::user_root`).
//! - [`billing`] is the "background work is free" predicate: free only while
//!   the backend's memory free period is on, answered by a cached
//!   `GET /memory/free-period`.
//!
//! Generic engine resolution, scope and the `cortexdb` engine stay in the core.
//! With nothing registered the core answers `tinyhumans` as unavailable.

pub(crate) mod billing;
pub(crate) mod engine;

use openhuman_embed::__host::memory_engines::{
    register_memory_engine, register_memory_health_probe, register_read_only_engine_factory,
    EngineEntry, EngineHealth, ReadOnlyEngineFactory, TINYHUMANS_ENGINE,
};
use std::sync::Arc;

use crate::install::InstallOptions;

/// Registers this area's hooks with the core. Idempotent.
pub(crate) fn register(_options: &InstallOptions) {
    register_memory_engine(
        TINYHUMANS_ENGINE,
        EngineEntry {
            build: Arc::new(engine::bind),
            free_background: Some(Arc::new(|config, bound| {
                Box::pin(billing::free_period_active(config, bound))
            })),
        },
    );
    let read_only_factory: ReadOnlyEngineFactory =
        Arc::new(|config| Box::pin(async move { engine::build_read_only_engine(&config) }));
    register_read_only_engine_factory(TINYHUMANS_ENGINE, read_only_factory);
    register_memory_health_probe(TINYHUMANS_ENGINE, Arc::new(read_only_health_probe));
    log::debug!("[tinyhumans] memory: hosted engine registered as '{TINYHUMANS_ENGINE}'");
}

/// Builds a throwaway engine for a read-only health request. It reads the
/// selected config's credential profile directly and never enters the normal
/// engine cache or publishes session-expiry state.
fn read_only_health_probe(
    config: openhuman_embed::__host::config::Config,
) -> openhuman_embed::__host::memory_engines::EngineFuture<EngineHealth> {
    Box::pin(async move {
        match engine::build_read_only_engine(&config) {
            Ok(engine) => engine.health().await,
            Err(reason) => EngineHealth::Down(reason.safe_message().to_owned()),
        }
    })
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
