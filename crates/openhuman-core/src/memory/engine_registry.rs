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

/// Safe failures while constructing an uncached memory engine for doctor live
/// checks. These variants intentionally carry no provider response text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyEngineError {
    ComponentDisabled,
    EngineDisabled,
    UnsupportedEngine,
    CredentialsUnavailable,
    EndpointUnavailable,
    InvalidConfiguration,
}

impl ReadOnlyEngineError {
    /// A safe operator-facing summary with no credential or provider detail.
    #[must_use]
    pub const fn safe_message(self) -> &'static str {
        match self {
            Self::ComponentDisabled => "Memory is disabled in the selected configuration.",
            Self::EngineDisabled => "Memory is turned off in the selected configuration.",
            Self::UnsupportedEngine => {
                "The selected memory engine is not available for inspection."
            }
            Self::CredentialsUnavailable => {
                "No usable credential is available for the selected memory engine."
            }
            Self::EndpointUnavailable => "The selected memory engine endpoint is unavailable.",
            Self::InvalidConfiguration => "The selected memory engine configuration is invalid.",
        }
    }
}

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

/// A read-only connectivity probe for the selected engine configuration.
/// Unlike [`EngineBuilder`], this hook must not consult ambient profile state,
/// mutate credential state, initialize storage, or cache the engine.
pub type ReadOnlyHealthProbe =
    Arc<dyn Fn(Config) -> EngineFuture<tinymemory_api::EngineHealth> + Send + Sync>;

/// Builds an uncached engine from the explicitly selected config using a
/// frozen, read-only credential snapshot. The returned engine may perform
/// operations requested by an opt-in live doctor check; its construction must
/// not consult ambient profile state or mutate credential/config stores.
pub type ReadOnlyEngineFactory = Arc<
    dyn Fn(Config) -> EngineFuture<Result<Arc<dyn MemoryEngine>, ReadOnlyEngineError>>
        + Send
        + Sync,
>;

/// A registered memory engine.
#[derive(Clone)]
pub struct EngineEntry {
    /// Binds the engine.
    pub build: EngineBuilder,
    /// Optional "background work is free" predicate; `None` means always free.
    pub free_background: Option<FreeBackground>,
}

static ENGINES: HookSlot<EngineEntry> = HookSlot::new("memory::engines");
static READ_ONLY_HEALTH_PROBES: HookSlot<ReadOnlyHealthProbe> =
    HookSlot::new("memory::read-only-health-probes");
static READ_ONLY_ENGINE_FACTORIES: HookSlot<ReadOnlyEngineFactory> =
    HookSlot::new("memory::read-only-engine-factories");

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

/// Registers a read-only health probe for a configured memory engine.
pub fn register_memory_health_probe(engine_id: impl Into<String>, probe: ReadOnlyHealthProbe) {
    READ_ONLY_HEALTH_PROBES.register(engine_id, probe);
}

/// Registers a read-only health probe for a context built from `extensions`.
pub fn register_memory_health_probe_in(
    extensions: &Extensions,
    engine_id: impl Into<String>,
    probe: ReadOnlyHealthProbe,
) {
    READ_ONLY_HEALTH_PROBES.register_in(extensions, engine_id, probe);
}

/// Registers a read-only, uncached engine constructor for the selected config.
pub fn register_read_only_engine_factory(
    engine_id: impl Into<String>,
    factory: ReadOnlyEngineFactory,
) {
    READ_ONLY_ENGINE_FACTORIES.register(engine_id, factory);
}

/// Registers a read-only, uncached engine constructor for one core context.
pub fn register_read_only_engine_factory_in(
    extensions: &Extensions,
    engine_id: impl Into<String>,
    factory: ReadOnlyEngineFactory,
) {
    READ_ONLY_ENGINE_FACTORIES.register_in(extensions, engine_id, factory);
}

/// Removes the process-wide memory engine `engine_id`.
pub fn unregister_memory_engine(engine_id: &str) -> bool {
    ENGINES.unregister(engine_id)
}

/// The registered entry for `engine_id`, if any.
pub(crate) fn lookup(engine_id: &str) -> Option<EngineEntry> {
    ENGINES.get(engine_id)
}

/// The read-only health probe for `engine_id`, if the current context has one.
pub(crate) fn read_only_health_probe(engine_id: &str) -> Option<ReadOnlyHealthProbe> {
    READ_ONLY_HEALTH_PROBES.get(engine_id)
}

pub(crate) fn read_only_engine_factory(engine_id: &str) -> Option<ReadOnlyEngineFactory> {
    READ_ONLY_ENGINE_FACTORIES.get(engine_id)
}

/// Wraps `engine` as a [`BoundEngine`] with the same write scrubbing every
/// configured engine gets. A registered builder returns
/// `Binding::On(bound_engine(..))`.
pub fn bound_engine(
    id: impl Into<String>,
    endpoint: impl Into<String>,
    engine: Arc<dyn MemoryEngine>,
) -> BoundEngine {
    let mut bound = super::engine::bind_external(engine, endpoint);
    bound.id = id.into();
    bound
}

#[cfg(test)]
#[path = "engine_registry_tests.rs"]
mod tests;
