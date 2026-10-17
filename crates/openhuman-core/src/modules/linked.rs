//! The registry of modules linked into the host executable.
//!
//! A `linked` source (`[modules] source = "linked"`, see [`super::source`])
//! serves a module from code compiled into the running binary instead of a
//! downloaded `cdylib`. The core cannot link module implementation crates, so
//! the code that does (`openhuman-modules-linked`, reachable only from the CLI
//! in local-development builds) hands the core a *factory* per module id with
//! [`register_linked`]. The factory returns tinybus' [`LinkedModule`]; the
//! resolver attaches it through `ModuleHost::attach_linked_with_config` with
//! the same private configuration a released module gets, so reinitialization
//! and config behave identically.
//!
//! The registry is empty in every binary that never calls [`register_linked`],
//! which is every shipped build; a `linked` request then falls back to the
//! release with a warning (see [`super::source::plan`]).
//!
//! # Process-global on purpose
//!
//! tinybus never unloads a module, and the module host itself is a process
//! singleton ([`super::host`]), so a process serves a given id from exactly one
//! source. The registry is likewise process-wide: it records what this binary
//! *can* link, not what any one configuration asked for.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use tinybus::module::LinkedModule;

/// Builds the linked module for one id. A plain `fn` so a registration carries
/// no state: the module crate's generated `linked_module` fits directly.
pub type LinkedFactory = fn() -> tinybus::Result<LinkedModule>;

static FACTORIES: Mutex<BTreeMap<&'static str, LinkedFactory>> = Mutex::new(BTreeMap::new());

fn factories() -> MutexGuard<'static, BTreeMap<&'static str, LinkedFactory>> {
    // A poisoned lock only means a panic elsewhere while holding it; the map
    // itself is never left half-updated by `insert`/`get`.
    FACTORIES
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// Register the in-process factory for the module registered as `id`.
///
/// Call once per module before the first load, normally from
/// `openhuman-modules-linked::install`. A repeat registration for the same id
/// replaces the earlier one (the last installer wins), which keeps `install`
/// idempotent.
pub fn register_linked(id: &'static str, factory: LinkedFactory) {
    let replaced = factories().insert(id, factory).is_some();
    log::debug!("[modules] linked factory registered id={id} replaced={replaced}");
}

/// Whether a factory is registered for `id`.
#[must_use]
pub fn is_registered(id: &str) -> bool {
    factories().contains_key(id)
}

/// The factory registered for `id`, if any.
#[must_use]
pub(super) fn factory_for(id: &str) -> Option<LinkedFactory> {
    factories().get(id).copied()
}

/// Ids with a registered factory, for diagnostics.
#[must_use]
pub fn registered_ids() -> Vec<&'static str> {
    factories().keys().copied().collect()
}

#[cfg(test)]
#[path = "linked_tests.rs"]
mod tests;
