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
    register_memory_engine, EngineEntry, TINYHUMANS_ENGINE,
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
    log::debug!("[tinyhumans] memory: hosted engine registered as '{TINYHUMANS_ENGINE}'");
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
