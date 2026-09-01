//! Per-workspace memory-driver binding — the memory subsystem's half of
//! `docs/specs/kernel.md` §3.1 (one driver per subsystem per process, per
//! workspace here), §3.4 (fail-closed trust), and §3.7 (a fallback is never
//! silent).
//!
//! ## Reached through [`CoreContext`], never through a global slot
//!
//! The binding is resolved by
//! [`CoreContext::memory_binding`](crate::core::runtime::CoreContext::memory_binding),
//! which keys on the context's workspace dir. The cache below is deliberately
//! shaped like the engine's `people::store::for_workspace`
//! — a **workspace-keyed map** — and deliberately *not* like the engine's
//! `global` slot, which is a single slot
//! holding "the one active-user workspace".
//!
//! That shape choice carries a real correctness property for free.
//! `memory::global::init` needs an explicit clear-on-failed-rebind guard so a
//! failed switch to workspace B cannot leave callers writing into workspace A.
//! With a workspace-keyed map there is no shared slot to go stale: a context
//! bound to B resolves the entry for B or falls back, and can never be handed
//! A's driver. Pinned by
//! `failed_bind_never_returns_previous_workspace_binding` in
//! `src/core/runtime/context.rs`.
//!
//! ## Two vocabularies meet here, on purpose
//!
//! [`crate::openhuman::memory::api`] is the host-owned memory contract: `MemoryProvider`,
//! `Capabilities`, `MemoryHealth`. [`crate::core::subsystem`] is the kernel's
//! *generic* driver vocabulary shared with the subsystems that come after
//! memory: `DriverClass`, `DriverCapabilities`, `DriverHealth`, `BoundDriver`.
//! This module is the adapter between them — the only place in the tree where
//! the conversion lives. `DriverClass` is reused from the kernel rather than
//! redefined here precisely because it is a *host* fact about how a driver was
//! bound, identical for every subsystem.
//!
//! The built-in driver is the compiled TinyMemory TinyBus module. The host no
//! longer exposes an embedded engine class for memory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::openhuman::memory::api::capabilities::Capabilities;
use crate::openhuman::memory::api::health::MemoryHealth;
use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::api::CONTRACT_VERSION;
use crate::openhuman::memory::guard::{GuardPolicy, MemoryGuard};
use tinymemory_api::null::{NullMemoryProvider, NULL_DRIVER_ID};

use crate::core::subsystem::{
    BoundDriver, DriverCapabilities, DriverClass, DriverHealth, SubsystemSlot,
};
use crate::openhuman::config::schema::MemorySubsystemConfig;

/// Registry id of the built-in TinyMemory module.
pub(crate) const MODULE_ID: &str = "tinymemory";

/// Why a bind fell back to the placeholder driver.
///
/// `reason` is operator-facing: it is logged, published on the event bus, and
/// rendered in status. It must therefore never interpolate `credential_ref` or
/// `endpoint` from [`crate::openhuman::config::schema::MemoryDriverConfig`],
/// which carries a manual redacting `Debug` for exactly that reason. Pinned by
/// `fallback_reason_never_contains_credential_ref_or_endpoint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackReason {
    /// The driver id that was asked for in `[subsystems.memory] driver`.
    pub configured_driver: String,
    /// Why it was refused.
    pub reason: String,
}

/// One bound memory driver, for one workspace.
pub struct MemoryBinding {
    provider: Arc<dyn MemoryProvider>,
    guard: Arc<MemoryGuard>,
    driver_id: String,
    /// The memory subtree this binding serves — `"memory"` for the shared tree,
    /// `"memory-<id>"` for a profile that opted into dedicated memory.
    memory_subdir: String,
    class: DriverClass,
    /// Asked **once**, at bind time, and cached here. The contract's
    /// `MemoryProvider::capabilities` doc is normative on this ("asked once at
    /// bind time and cached"): re-asking would let a driver's advertised
    /// surface drift underneath an already-filtered RPC/tool registration.
    capabilities: Capabilities,
    fallback: Option<FallbackReason>,
}

impl MemoryBinding {
    /// The bound driver.
    pub fn provider(&self) -> &Arc<dyn MemoryProvider> {
        &self.provider
    }

    pub(crate) fn unguarded_provider(&self) -> &Arc<dyn MemoryProvider> {
        &self.provider
    }

    pub fn guard(&self) -> Arc<MemoryGuard> {
        Arc::clone(&self.guard)
    }

    pub fn disables_memory(&self) -> bool {
        self.class == DriverClass::Null && self.fallback.is_none()
    }

    /// The id of the driver that actually bound — `"null"` after a fallback,
    /// not the id that was asked for (that is in [`Self::fallback`]).
    pub fn driver_id(&self) -> &str {
        &self.driver_id
    }

    /// The memory subtree this binding resolved to.
    ///
    /// `"memory"` is the shared tree; `"memory-<id>"` is a profile that opted
    /// into dedicated memory, and keeping the two apart is what makes
    /// `dedicatedMemory` isolation hold.
    ///
    /// Worth an accessor because the routing decision is made **here**, at bind
    /// time, but only reaches disk lazily: a module-backed driver opens the
    /// subtree on its first call (`OpenStore`), so nothing observes the choice
    /// until memory is actually used. Callers that need to report or assert
    /// which tree they were bound to — status output, and the session-builder
    /// tests — have no other way to see it.
    pub fn memory_subdir(&self) -> &str {
        &self.memory_subdir
    }

    /// How the bound driver was reached. A host fact, never self-reported.
    pub fn class(&self) -> DriverClass {
        self.class
    }

    /// The cached capability set. Cheap: `Capabilities` is a `Copy` bitset.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// `Some` when this binding is a fallback; `None` when the configured
    /// driver bound as asked.
    pub fn fallback(&self) -> Option<&FallbackReason> {
        self.fallback.as_ref()
    }

    /// This binding in the kernel's generic vocabulary, for the subsystem
    /// registry and `subsystems_status` (kernel.md §6 item 6). This is the
    /// memory adapter `core::subsystem`'s module docs said would land later.
    pub fn to_bound_driver(&self) -> BoundDriver {
        BoundDriver {
            slot: SubsystemSlot::Memory,
            id: self.driver_id.clone(),
            class: self.class,
            capabilities: to_driver_capabilities(self.capabilities),
            health: DriverHealth::Ready,
            contract_version: CONTRACT_VERSION,
            fell_back_from: self.fallback.as_ref().map(|f| f.configured_driver.clone()),
        }
    }
}

/// Convert the memory contract's typed capability set into the kernel's opaque
/// one. The kernel deliberately does not know memory's family vocabulary.
pub fn to_driver_capabilities(capabilities: Capabilities) -> DriverCapabilities {
    capabilities.iter().map(|c| c.as_str()).collect()
}

/// Convert the memory contract's health into the kernel's. A total three-arm
/// match, which is why both enums were shaped one-for-one.
pub fn to_driver_health(health: MemoryHealth) -> DriverHealth {
    match health {
        MemoryHealth::Ready => DriverHealth::Ready,
        MemoryHealth::Degraded { reason } => DriverHealth::Degraded { reason },
        MemoryHealth::Down { reason } => DriverHealth::Down { reason },
    }
}

/// The capability set assumed when nothing is bound.
///
/// **Deliberately the full set.** This mirrors
/// [`crate::core::all`]'s `group_allowed`, which returns `true` when there is
/// no ambient context: roughly 4000 unit tests run pre-boot with no bound
/// driver, and a deny-by-default here would fail all of them at once. Denying a
/// capability is only ever correct *after* a driver has actually answered
/// `capabilities()`.
pub fn unbound_default_capabilities() -> Capabilities {
    Capabilities::all()
}

/// Decide, from config alone, whether the configured driver may bind.
///
/// Pure — no I/O, no globals — so the fail-closed trust rule is unit-testable
/// without booting anything.
///
/// # Errors
///
/// Returns the [`FallbackReason`] to record and publish when the configured
/// driver is refused. Callers fall back rather than failing: kernel.md §3.7
/// requires the subsystem stay bound, loudly.
pub fn admit(cfg: &MemorySubsystemConfig) -> Result<(String, DriverClass), FallbackReason> {
    let configured_id = cfg.driver.trim();
    if configured_id.is_empty() {
        return Err(FallbackReason {
            configured_driver: String::new(),
            reason: "[subsystems.memory] driver is empty".to_string(),
        });
    }

    let refuse = |reason: &str| FallbackReason {
        configured_driver: configured_id.to_string(),
        reason: reason.to_string(),
    };

    // Temporary persisted-config alias. The schema still comes from the
    // legacy contract until its remaining engine callers are moved onto the
    // host-owned copy; both values bind the compiled module and report its
    // actual id. Remove this alias with that final schema cutover.
    const LEGACY_MODULE_ID: &str = "tinycortex";
    let id = if configured_id == LEGACY_MODULE_ID {
        MODULE_ID
    } else {
        configured_id
    };

    // The two built-ins need no `[subsystems.memory.drivers.<id>]` entry.
    let Some(entry) = cfg
        .drivers
        .get(configured_id)
        .or_else(|| cfg.drivers.get(id))
    else {
        return match id {
            NULL_DRIVER_ID => Ok((id.to_string(), DriverClass::Null)),
            MODULE_ID => Ok((id.to_string(), DriverClass::Module)),
            _ => Err(refuse(&format!(
                "driver '{id}' is not built in; add [subsystems.memory.drivers.{id}] with an explicit class line"
            ))),
        };
    };

    let class = match entry.class.as_deref() {
        None if id == NULL_DRIVER_ID => DriverClass::Null,
        None if id == MODULE_ID => DriverClass::Module,
        None => {
            return Err(refuse(&format!(
                "driver '{id}' is not built in and requires an explicit class line"
            )))
        }
        Some(raw) => DriverClass::parse(raw).map_err(|e| refuse(&e))?,
    };

    if class == DriverClass::Embedded {
        return Err(refuse(
            "embedded memory drivers are no longer supported; use the 'tinymemory' module driver",
        ));
    }

    let built_in_class = match id {
        NULL_DRIVER_ID => Some(DriverClass::Null),
        MODULE_ID => Some(DriverClass::Module),
        _ => None,
    };
    if let Some(expected) = built_in_class {
        if class != expected {
            return Err(refuse(&format!(
                "built in driver '{configured_id}' has class '{expected}' and cannot be re-classed as '{class}'"
            )));
        }
    }

    if class == DriverClass::Module && id != MODULE_ID {
        return Err(refuse(&format!(
            "module driver '{id}' is not registered; the built-in memory module id is '{MODULE_ID}'"
        )));
    }

    if class == DriverClass::External {
        // kernel.md §3.4: fail-closed. Trust must be explicitly raised before
        // an out-of-process driver is allowed to answer for memory.
        if entry.trust_state != "trusted" {
            return Err(refuse(
                "external driver is untrusted: set trust_state = \"trusted\" \
                 under [subsystems.memory.drivers] to allow this binding",
            ));
        }
        // Distinct reason string from the trust refusal above, so the trust
        // test cannot pass for the wrong reason.
        return Err(refuse(
            "external driver transport is not implemented yet (the http adapter lands in M4)",
        ));
    }

    Ok((id.to_string(), class))
}

/// Build the binding for a workspace. Infallible by design: an inadmissible
/// driver falls back to the placeholder rather than leaving the slot empty
/// (kernel.md §3.7 — "logged loudly, surfaced in status, never silent").
fn build(workspace_dir: &Path, memory_subdir: &str, cfg: &MemorySubsystemConfig) -> MemoryBinding {
    match admit(cfg) {
        Ok((driver_id, class)) => {
            let (provider, reported_class): (Arc<dyn MemoryProvider>, DriverClass) =
                if class == DriverClass::Null {
                    (Arc::new(NullMemoryProvider::new()), DriverClass::Null)
                } else {
                    module_provider(workspace_dir, memory_subdir)
                };
            let binding = bind_provider(
                provider,
                driver_id,
                memory_subdir.to_string(),
                reported_class,
                None,
            );
            log::info!(
                "[memory:binding] workspace={} bound driver='{}' class={} capabilities=[{}]",
                workspace_dir.display(),
                binding.driver_id(),
                binding.class(),
                binding
                    .capabilities()
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            binding
        }
        Err(fallback) => {
            log::warn!(
                "[memory:binding] workspace={} driver '{}' refused to bind ({}); \
                 falling back to '{NULL_DRIVER_ID}' — memory writes are DISCARDED this run",
                workspace_dir.display(),
                fallback.configured_driver,
                fallback.reason
            );
            // Sync, and a no-op when the bus is not yet initialized, so this is
            // safe to call pre-boot with no `#[cfg(test)]` guard.
            crate::core::bus::BUS.publish(
                crate::core::events::DomainEvent::MemoryDriverBindFailed {
                    configured_driver: fallback.configured_driver.clone(),
                    bound_driver: NULL_DRIVER_ID.to_string(),
                    reason: fallback.reason.clone(),
                },
            );
            bind_provider(
                Arc::new(NullMemoryProvider::new()),
                NULL_DRIVER_ID.to_string(),
                memory_subdir.to_string(),
                DriverClass::Null,
                Some(fallback),
            )
        }
    }
}

#[cfg(all(feature = "modules", not(test)))]
fn module_provider(
    _workspace_dir: &Path,
    memory_subdir: &str,
) -> (Arc<dyn MemoryProvider>, DriverClass) {
    // The workspace itself still comes from the boot policy — the module is
    // loaded once per process and captures it at setup. The **subtree** is per
    // binding, and the module opens it on first use.
    (
        Arc::new(
            crate::openhuman::modules::memory::ModuleMemoryProvider::from_boot_policy()
                .in_subdir(memory_subdir),
        ),
        DriverClass::Module,
    )
}

#[cfg(all(feature = "modules", test))]
fn module_provider(
    _workspace_dir: &Path,
    memory_subdir: &str,
) -> (Arc<dyn MemoryProvider>, DriverClass) {
    // Unit tests do not run the full boot sequence that publishes the module
    // policy. A native module is loaded once per process and therefore captures
    // the first workspace it receives. Pin every test binding to the same
    // workspace as the process-global test client so concurrent tests cannot
    // win module initialization with an unrelated tempdir and split guarded
    // writes from legacy read-back calls.
    let workspace_dir = crate::openhuman::memory::ops::shared_memory_test_workspace();
    let mut config = crate::openhuman::config::Config::default();
    config.workspace_dir = workspace_dir.clone();
    config.modules.install_dir = Some(workspace_dir.join("modules").to_string_lossy().into_owned());
    if let Some(path) = std::env::var_os("TINYMEMORY_TEST_MODULE") {
        config
            .modules
            .overrides
            .push(crate::openhuman::config::schema::ModuleOverride {
                id: MODULE_ID.to_string(),
                path: path.to_string_lossy().into_owned(),
            });
    }
    (
        Arc::new(
            crate::openhuman::modules::memory::ModuleMemoryProvider::new(Arc::new(config))
                .in_subdir(memory_subdir),
        ),
        DriverClass::Module,
    )
}

#[cfg(not(feature = "modules"))]
fn module_provider(
    _workspace_dir: &Path,
    _memory_subdir: &str,
) -> (Arc<dyn MemoryProvider>, DriverClass) {
    log::warn!(
        "[memory:binding] the 'modules' feature is disabled; binding the null memory provider"
    );
    (Arc::new(NullMemoryProvider::new()), DriverClass::Null)
}

/// The single place `capabilities()` is asked. Every construction path — real
/// bind, fallback, and the test seam — goes through here, so the "asked once
/// per bind" property holds by construction rather than by convention.
fn bind_provider(
    provider: Arc<dyn MemoryProvider>,
    driver_id: String,
    memory_subdir: String,
    class: DriverClass,
    fallback: Option<FallbackReason>,
) -> MemoryBinding {
    let capabilities = provider.capabilities();
    let guard = Arc::new(MemoryGuard::new(
        Arc::clone(&provider),
        Arc::new(GuardPolicy::new(
            driver_id.clone(),
            class,
            crate::openhuman::config::schema::MemoryHooksConfig::default(),
            "trusted",
        )),
    ));
    MemoryBinding {
        provider,
        guard,
        driver_id,
        memory_subdir,
        class,
        capabilities,
        fallback,
    }
}

/// Test-only injection seam: bind an arbitrary provider through the same
/// ask-once-and-cache path [`build`] uses. Exists because [`build`] hard-codes
/// the placeholder, so the "capabilities asked exactly once" property would
/// otherwise be untestable.
#[cfg(test)]
pub(crate) fn bind_provider_for_test(
    provider: Arc<dyn MemoryProvider>,
    class: DriverClass,
) -> MemoryBinding {
    let driver_id = provider.driver_id().to_string();
    bind_provider(provider, driver_id, "memory".to_string(), class, None)
}

/// Per-workspace binding cache. Same shape as
/// `memory::people::store::STORES` — see the module docs for why this is a map
/// and not a slot.
/// Keyed by workspace **and memory subtree**: a profile that opted into
/// dedicated memory is a different store, so it must be a different binding.
/// The subtree is `"memory"` for every ordinary caller.
type BindingCacheKey = (PathBuf, String, MemorySubsystemConfig);
static BINDINGS: OnceLock<RwLock<HashMap<BindingCacheKey, Arc<MemoryBinding>>>> = OnceLock::new();

/// The bound memory driver for `workspace_dir`, constructing it on first use.
///
/// The same workspace always resolves to the same cached `Arc` (so
/// `capabilities()` is asked once); different workspaces get isolated bindings.
///
/// # Errors
///
/// Only lock poisoning. A driver that cannot bind is *not* an error here — it
/// falls back, per kernel.md §3.7.
pub fn for_workspace(
    workspace_dir: &Path,
    cfg: &MemorySubsystemConfig,
) -> Result<Arc<MemoryBinding>, String> {
    for_subtree(workspace_dir, "memory", cfg)
}

/// A driver that reports the diagnostics it was handed, and does nothing else.
///
/// Reads that used to hit the engine's tables go through the contract now, and
/// the real driver is a compiled module that cannot load inside a unit test —
/// so a test workspace binds the null driver and every diagnostic answers
/// empty. A handler that used to be provable by writing rows and calling it
/// needs a driver in between.
///
/// The split that leaves is the honest one. What a handler *derives* from the
/// numbers is the host's rule and belongs in the host's tests, which is what
/// this exists for. What a given store *is* — that an ingest raises the chunk
/// count, that a deferred job stays ready without becoming eligible — is the
/// driver's rule, pinned in the driver's own conformance suite against a real
/// store.
///
/// Everything outside `Maintenance` delegates to the null driver: a test that
/// needed those would be testing something this double is the wrong shape for.
#[cfg(test)]
pub(crate) struct FixedDiagnostics {
    inner: NullMemoryProvider,
    /// How many times the host has asked this driver to retry failed work,
    /// and how many jobs it should say it requeued when asked.
    ///
    /// The gate in front of the ask is host logic — only an embedder change
    /// should un-park anything — so a test needs to see whether the ask
    /// happened, separately from what the driver would have done.
    retry_calls: std::sync::atomic::AtomicUsize,
    retry_requeues: u64,
    /// How many times the host has asked this driver to re-embed.
    ///
    /// `reembed` enqueues work rather than doing it, so the host's side of that
    /// contract is only that it *asked* — whether a row appears is the driver's
    /// business, and pinning it here would test the driver through the host.
    reembed_calls: std::sync::atomic::AtomicUsize,
    store: crate::openhuman::memory::api::provider::types::StoreStats,
    queue: crate::openhuman::memory::api::provider::types::QueueStats,
    failure: Option<crate::openhuman::memory::api::provider::types::QueueFailure>,
    /// What this driver says about a backfill running in its process.
    ///
    /// Separate from [`Self::queue`] on purpose, mirroring the contract: the
    /// flag is not derivable from the counts, and a test that needs the gap
    /// between them — nothing ready, nothing running, backfill unfinished —
    /// has to set the two independently.
    backfill: bool,
    /// What [`MemoryMaintenance::flush_pending`] answers, when a test sets it.
    flush: crate::openhuman::memory::api::provider::types::FlushOutcome,
    /// What [`MemoryMaintenance::reset_derived_index`] answers, likewise.
    reset: crate::openhuman::memory::api::provider::types::ResetOutcome,
}

#[cfg(test)]
#[path = "binding_fixed_diagnostics_impl_tests.rs"]
mod fixed_diagnostics_impl;

/// Bind a driver reporting fixed diagnostics as this workspace's driver.
///
/// The shorthand every test needs that reaches a handler reading through
/// `Maintenance`. Without a binding installed, resolving one attempts to load
/// the compiled module, and in a test process that can block rather than
/// fail — the module host's runtime belongs to whichever test created it, so
/// a later test finds a broker whose tasks are already gone.
#[cfg(test)]
pub(crate) fn install_diagnostics_for_test(
    workspace_dir: &Path,
    cfg: &MemorySubsystemConfig,
    store: crate::openhuman::memory::api::provider::types::StoreStats,
    queue: crate::openhuman::memory::api::provider::types::QueueStats,
) -> Arc<FixedDiagnostics> {
    let driver = Arc::new(FixedDiagnostics::new(store, queue));
    install_for_test(
        workspace_dir,
        cfg,
        Arc::clone(&driver) as Arc<dyn MemoryProvider>,
    );
    driver
}

/// Bind a driver that reports `requeued` from `retry_failed` and counts the
/// asks, for tests about *when* the host asks rather than what a queue does.
#[cfg(test)]
pub(crate) fn install_retrying_driver_for_test(
    config: &crate::openhuman::config::Config,
    requeued: u64,
) -> Arc<FixedDiagnostics> {
    let driver = Arc::new(
        FixedDiagnostics::new(Default::default(), Default::default()).requeueing(requeued),
    );
    install_for_test(
        &config.workspace_dir,
        &config.subsystems.memory,
        Arc::clone(&driver) as Arc<dyn MemoryProvider>,
    );
    driver
}

/// Install `provider` as the binding a workspace resolves to.
///
/// The cache below is normally filled by [`build`], which binds the compiled
/// TinyMemory module — and that module is not loadable inside a unit test, so
/// a test workspace otherwise resolves to the null driver and every read
/// through the contract answers empty.
///
/// That matters more since reads moved off the engine: a handler that used to
/// be provable by writing rows and calling it now needs a driver in between.
/// This is the seam that puts one there.
///
/// Test-only. It writes a process-global map, so a test using it must own the
/// workspace path it installs against — which a `tempdir` does.
#[cfg(test)]
pub(crate) fn install_for_test(
    workspace_dir: &Path,
    cfg: &MemorySubsystemConfig,
    provider: Arc<dyn MemoryProvider>,
) {
    let binding = Arc::new(bind_provider_for_test(provider, DriverClass::Module));
    let key = (
        workspace_dir.to_path_buf(),
        "memory".to_string(),
        cfg.clone(),
    );
    BINDINGS
        .get_or_init(Default::default)
        .write()
        .expect("binding cache lock")
        .insert(key, binding);
}

/// The bound memory driver for the workspace a whole [`Config`] names.
///
/// The two pieces [`for_workspace`] needs sit in different halves of `Config`,
/// so most call sites were spelling the same pair out. It is the same cached
/// binding either way.
///
/// [`Config`]: crate::openhuman::config::Config
///
/// # Errors
///
/// Only lock poisoning, as [`for_workspace`].
pub fn for_config(config: &crate::openhuman::config::Config) -> Result<Arc<MemoryBinding>, String> {
    for_workspace(&config.workspace_dir, &config.subsystems.memory)
}

/// The bound memory driver for one **memory subtree** of `workspace_dir`.
///
/// `"memory"` is the shared tree and is what [`for_workspace`] passes;
/// `"memory-<id>"` is a profile that opted into dedicated memory. Each subtree
/// gets its own binding and therefore its own driver, which is the whole point
/// — two profiles with dedicated memory must not see each other's entries.
///
/// # Errors
///
/// Only lock poisoning, as [`for_workspace`].
pub fn for_subtree(
    workspace_dir: &Path,
    memory_subdir: &str,
    cfg: &MemorySubsystemConfig,
) -> Result<Arc<MemoryBinding>, String> {
    let cache = BINDINGS.get_or_init(Default::default);
    let key = (
        workspace_dir.to_path_buf(),
        memory_subdir.to_string(),
        cfg.clone(),
    );
    if let Some(binding) = cache
        .read()
        .map_err(|e| format!("[memory:binding] cache read lock poisoned: {e}"))?
        .get(&key)
    {
        return Ok(Arc::clone(binding));
    }

    let binding = Arc::new(build(workspace_dir, memory_subdir, cfg));

    let mut guard = cache
        .write()
        .map_err(|e| format!("[memory:binding] cache write lock poisoned: {e}"))?;
    // Re-check under the write lock: a racing caller may have bound the same
    // workspace while we were building. Reuse theirs so one workspace never has
    // two live drivers (kernel.md §3.1) and `capabilities()` stays asked once.
    let entry = guard.entry(key).or_insert_with(|| Arc::clone(&binding));
    Ok(Arc::clone(entry))
}

#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
