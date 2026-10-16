//! Registered memory engines, keyed by engine id.
//!
//! [`engine::resolve`](super::engine::resolve) builds the engines the core
//! knows (`cortexdb`, `none`). An outside crate registers another under its own
//! id with [`register_memory_engine`]: the builder turns the `[memory]` config
//! (and the layout root) into a [`Binding`], and an optional predicate says
//! whether background memory work is free for it.
//!
//! Unlike [`install_host_engine`](super::engine::install_host_engine), which
//! binds one engine for the whole process and is ignored under SaaS, a
//! registered engine is resolved per config like any configured engine, so it
//! is admitted for every tenant: the builder receives that tenant's config and
//! root. Registered ids win over the built-in match, and with nothing
//! registered the engine selection and the free-period check are unchanged.
//!
//! Free background work ([`free_period_active`](super::billing::free_period_active)):
//! an engine with no registration is free, a registered engine is free unless
//! its predicate says otherwise (a registration without one is free), and an
//! unbound engine is not free.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tinymemory_api::MemoryEngine;

use super::engine::{Binding, BoundEngine};
use crate::config::Config;
use crate::core::slots::{Extensions, HookSlot};

/// A boxed `Send` future.
pub type EngineFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What a registered engine builder is asked to bind.
pub struct EngineRequest<'a> {
    /// The tenant's configuration.
    pub config: &'a Config,
    /// The layout root: `None` the legacy tree, `Some("user:<id>")` the
    /// person's subtree.
    pub root: Option<&'a str>,
}

/// Binds the engine for a config and layout root.
pub type EngineBuilder = Arc<dyn for<'a> Fn(&EngineRequest<'a>) -> Binding + Send + Sync>;

/// Whether background memory work is free for the bound engine and config.
pub type FreeBackground = Arc<dyn Fn(Config, BoundEngine) -> EngineFuture<bool> + Send + Sync>;

/// A registered memory engine.
#[derive(Clone)]
pub struct EngineEntry {
    /// Binds the engine.
    pub build: EngineBuilder,
    /// Optional "background work is free" predicate; `None` means always free.
    pub free_background: Option<FreeBackground>,
}

static ENGINES: HookSlot<EngineEntry> = HookSlot::new("memory::engines");

/// Registers the memory engine `engine_id` for the whole process.
pub fn register_memory_engine(engine_id: impl Into<String>, entry: EngineEntry) {
    ENGINES.register(engine_id, entry);
}

/// Registers the memory engine `engine_id` for the context built from
/// `extensions`.
pub fn register_memory_engine_in(
    extensions: &Extensions,
    engine_id: impl Into<String>,
    entry: EngineEntry,
) {
    ENGINES.register_in(extensions, engine_id, entry);
}

/// Removes the process-wide memory engine `engine_id`.
pub fn unregister_memory_engine(engine_id: &str) -> bool {
    ENGINES.unregister(engine_id)
}

/// The registered entry for `engine_id`, if any.
pub(crate) fn lookup(engine_id: &str) -> Option<EngineEntry> {
    ENGINES.get(engine_id)
}

/// Wraps `engine` as a [`BoundEngine`] with the same write scrubbing every
/// configured engine gets. A registered builder returns
/// `Binding::On(bound_engine(..))`.
pub fn bound_engine(
    id: impl Into<String>,
    endpoint: impl Into<String>,
    engine: Arc<dyn MemoryEngine>,
) -> BoundEngine {
    BoundEngine {
        id: id.into(),
        endpoint: endpoint.into(),
        engine: super::guard::ScrubbingEngine::wrap(engine),
    }
}

#[cfg(test)]
#[path = "engine_registry_tests.rs"]
mod tests;
