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
//! shaped like
//! [`memory::people::store::for_workspace`](crate::openhuman::memory::people::store::for_workspace)
//! — a **workspace-and-config-keyed map** — and deliberately *not* like
//! [`memory::global`](crate::openhuman::memory::global), which is a single slot
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
//! [`tinycortex_api`] is the *memory contract*: `MemoryProvider`,
//! `Capabilities`, `MemoryHealth`. [`crate::core::subsystem`] is the kernel's
//! *generic* driver vocabulary shared with the subsystems that come after
//! memory: `DriverClass`, `DriverCapabilities`, `DriverHealth`, `BoundDriver`.
//! This module is the adapter between them — the only place in the tree where
//! the conversion lives. `DriverClass` is reused from the kernel rather than
//! redefined here precisely because it is a *host* fact about how a driver was
//! bound, identical for every subsystem.
//!
//! ## Scope of this step (M3d)
//!
//! The [`DriverClass::Embedded`] arm of [`build`] binds the real
//! [`EmbeddedMemoryProvider`], which wraps the in-process tinycortex engine.
//! [`DriverClass::Null`] still binds [`NullMemoryProvider`] — an operator who
//! wrote `driver = "null"` asked for `/dev/null` and must get it — and so does
//! every fallback.
//!
//! The embedded driver now implements **all thirteen** families, so a bound
//! context and an unbound one advertise the same set. That was the whole point
//! of M3: before it, binding *narrowed* the advertised set from thirteen
//! families to the null placeholder's three, which made gating anything on
//! `memory_capabilities()` actively dangerous. It is now safe, and M4 is where
//! that gating lands.
//!
//! A fallback binding still advertises only the mandatory three, because a
//! fallback really is the null placeholder — that is the honest answer, not a
//! leftover.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use tinycortex_api::capabilities::Capabilities;
use tinycortex_api::health::MemoryHealth;
use tinycortex_api::null::{NullMemoryProvider, NULL_DRIVER_ID};
use tinycortex_api::provider::MemoryProvider;
use tinycortex_api::CONTRACT_VERSION;

use tinymemory::registry::{
    ConfigLabels, DriverClass as ContractDriverClass, DriverEntry, DriverRegistry,
};

use crate::core::subsystem::{
    BoundDriver, DriverCapabilities, DriverClass, DriverHealth, SubsystemSlot,
};
use crate::openhuman::memory::driver::embedded::EmbeddedMemoryProvider;
use crate::openhuman::memory::guard::{GuardPolicy, MemoryGuard};
use tinymemory_api::host::MemoryHooksConfig;
use tinymemory_api::host::MemorySubsystemConfig;

/// Why a bind fell back to the placeholder driver.
///
/// Defined in [`tinymemory::registry`] alongside the admission rules that
/// produce it. `reason` is operator-facing: it is logged, published on the
/// event bus, and rendered in status, so it must never interpolate
/// `credential_ref` or `endpoint` from
/// [`tinymemory_api::host::MemoryDriverConfig`], which carries a
/// manual redacting `Debug` for exactly that reason. The crate enforces this
/// structurally — [`DriverEntry`] carries neither field, so a refusal built
/// there cannot reach one. Pinned by
/// `fallback_reason_never_contains_credential_ref_or_endpoint`.
pub use tinymemory::registry::FallbackReason;

/// One bound memory driver, for one workspace.
pub struct MemoryBinding {
    provider: Arc<dyn MemoryProvider>,
    /// The policy decorator over [`Self::unguarded_provider`] — the handle product code
    /// receives, via `CoreContext::memory()`. Built here rather than by each
    /// caller so "every caller gets a guarded handle" holds by construction,
    /// the same way `capabilities()` is asked exactly once by construction.
    guard: Arc<MemoryGuard>,
    driver_id: String,
    class: DriverClass,
    /// Asked **once**, at bind time, and cached here. The contract's
    /// `MemoryProvider::capabilities` doc is normative on this ("asked once at
    /// bind time and cached"): re-asking would let a driver's advertised
    /// surface drift underneath an already-filtered RPC/tool registration.
    capabilities: Capabilities,
    fallback: Option<FallbackReason>,
}

impl MemoryBinding {
    /// The bound driver, **unguarded**.
    ///
    /// Retained for identity/health/status, which are liveness probes rather
    /// than product code (`memory::ops::provider` is the one production
    /// caller). New call sites want [`Self::guard`] — see
    /// `CoreContext::memory()`.
    ///
    /// Named `unguarded_provider` rather than `provider` on purpose. The
    /// enforcement lint in `memory::bypass_allowlist_tests` matches text, and a
    /// `.provider(` needle would over-match `TaskSourceFilter::provider()` and
    /// `ModelRef::provider()` — six junk allowlist entries, which is exactly
    /// the rot `bypass_allowlist_has_no_stale_entries` exists to prevent. A
    /// distinctive name gives the lint a needle with no false positives, and
    /// puts the hazard in the reader's face at the call site.
    ///
    /// Visibility is narrowed to the memory family so the lint's text match is
    /// backed by a *compiler*-enforced boundary: even if `MemoryBinding` grows
    /// another reachable path, no module outside `openhuman::memory` can name
    /// this accessor at all.
    pub(crate) fn unguarded_provider(&self) -> &Arc<dyn MemoryProvider> {
        &self.provider
    }

    /// The guarded driver — the only handle product code should hold
    /// (`docs/specs/kernel.md` §3.4).
    pub fn guard(&self) -> Arc<MemoryGuard> {
        Arc::clone(&self.guard)
    }

    /// The id of the driver that actually bound — `"null"` after a fallback,
    /// not the id that was asked for (that is in [`Self::fallback`]).
    pub fn driver_id(&self) -> &str {
        &self.driver_id
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

    /// Whether the operator asked for memory to be **off**.
    ///
    /// True only for a deliberate `[subsystems.memory] driver = "null"` — the
    /// class alone is not enough, because a *fallback* also binds the null
    /// placeholder and a misconfiguration must not silently take memory away
    /// with it. A fallback is loud (`fallback()` is `Some`, status reports it)
    /// and keeps the surface present.
    ///
    /// Read by [`CoreContext::memory_capabilities`](crate::core::runtime::context::CoreContext::memory_capabilities),
    /// which answers with the empty set here, so the memory RPC methods and
    /// memory agent tools are **absent** rather than present-and-answering off
    /// some other store. That matters because most memory handlers still reach
    /// the engine directly through `active_memory_client()` — the guarded
    /// re-point is incremental and tracked in
    /// `docs/specs/memory-guard-allowlist.md` — so leaving the surface
    /// registered under a null binding would read the embedded SQLite store an
    /// operator believed they had turned off.
    pub fn disables_memory(&self) -> bool {
        self.class == DriverClass::Null && self.fallback.is_none()
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

/// The registry of driver ids whose class this host fixes.
///
/// [`DriverRegistry::builtin`] already reserves `null` and `tinycortex`, which
/// are exactly this host's two built-in ids — so the builtin set is used as-is
/// rather than re-declared. A host bundling an adapter the crate does not know
/// about would add it here with `with_reserved`.
fn registry() -> DriverRegistry {
    DriverRegistry::builtin()
}

/// The config-path spellings quoted back to the operator in refusal messages.
///
/// The crate does not know what this host's config file looks like; these are
/// the blocks an operator would actually edit.
const CONFIG_LABELS: ConfigLabels<'static> = ConfigLabels {
    section: "[subsystems.memory]",
    drivers: "[subsystems.memory.drivers]",
    driver_entry: "[subsystems.memory.drivers.<id>]",
};

/// The class a built-in driver id is *fixed* to, or `None` for any other id.
///
/// Both built-in ids name one specific implementation, so the registry is the
/// authority for their class in every path — the implicit one and the explicit
/// `class = …` line, which may only confirm what this returns.
pub(crate) fn reserved_class(id: &str) -> Option<DriverClass> {
    registry().reserved_class(id).map(from_contract_class)
}

/// The contract's driver class in the kernel's generic vocabulary.
///
/// A total three-arm match, which is why both enums were shaped one-for-one.
/// The kernel's own enum is deliberately not replaced by the contract's: it is
/// shared with the subsystems that come after memory, which must not inherit
/// their vocabulary from a *memory* crate.
fn from_contract_class(class: ContractDriverClass) -> DriverClass {
    match class {
        ContractDriverClass::Embedded => DriverClass::Embedded,
        ContractDriverClass::External => DriverClass::External,
        ContractDriverClass::Null => DriverClass::Null,
    }
}

/// Decide, from config alone, whether the configured driver may bind.
///
/// Pure — no I/O, no globals — so the fail-closed trust rule is unit-testable
/// without booting anything.
///
/// The rules themselves live in [`tinymemory::registry`], because they are the
/// one part of binding with the same correct answer for every host: a built-in
/// id's class is fixed and an explicit `class` line may only confirm it, an
/// unknown id is refused rather than guessed, and an external driver is
/// fail-closed on trust. This function is the projection from *this* host's
/// config shape onto that decision, and the conversion back into the kernel's
/// generic driver vocabulary.
///
/// Note what is deliberately **not** passed to the crate: only the `class` and
/// `trust_state` of the driver entry cross, never `credential_ref` or
/// `endpoint`. A refusal message is operator-facing and logged, so the narrow
/// projection is what makes "no secret can appear in a refusal" structural
/// rather than a rule someone has to remember.
///
/// # Errors
///
/// Returns the [`FallbackReason`] to record and publish when the configured
/// driver is refused. Callers fall back rather than failing: kernel.md §3.7
/// requires the subsystem stay bound, loudly.
pub fn admit(cfg: &MemorySubsystemConfig) -> Result<(String, DriverClass), FallbackReason> {
    let id = cfg.driver.trim();
    let entry = cfg.drivers.get(id).map(|entry| DriverEntry {
        class: entry.class.as_deref(),
        trust_state: entry.trust_state.as_str(),
    });

    let admission = registry().admit(&cfg.driver, entry, CONFIG_LABELS)?;
    Ok((admission.id, from_contract_class(admission.class)))
}

/// Build the binding for a workspace. Infallible by design: an inadmissible
/// driver falls back to the placeholder rather than leaving the slot empty
/// (kernel.md §3.7 — "logged loudly, surfaced in status, never silent").
fn build(workspace_dir: &Path, cfg: &MemorySubsystemConfig) -> MemoryBinding {
    match admit(cfg) {
        Ok((driver_id, class)) => {
            let provider: Arc<dyn MemoryProvider> = match class {
                // Construction is deliberately sync and I/O-free: this runs on
                // `CoreContext::memory_binding`, which ~4000 pre-boot tests
                // call with no tokio runtime. The driver resolves its client on
                // first use — see `driver::embedded`'s module docs.
                DriverClass::Embedded => {
                    Arc::new(EmbeddedMemoryProvider::new(workspace_dir, cfg.hooks))
                }
                DriverClass::Null => Arc::new(NullMemoryProvider::new()),
                // Unreachable: `admit` refuses every external driver above, so
                // this arm cannot bind a transport that does not exist yet.
                DriverClass::External => Arc::new(NullMemoryProvider::new()),
            };
            // The configured trust state for the driver that actually bound.
            // Absent `[subsystems.memory.drivers.<id>]` entry ⇒ the fail-closed
            // default, which only ever matters for an external class.
            let trust_state = cfg
                .drivers
                .get(&driver_id)
                .map(|entry| entry.trust_state.clone())
                .unwrap_or_else(|| crate::openhuman::memory::guard::policy::TRUSTED.to_string());
            let binding = bind_provider(provider, driver_id, class, cfg.hooks, trust_state, None);
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
            crate::openhuman::memory::events::publish(
                crate::openhuman::memory::events::MemoryEvent::DriverBindFailed {
                    configured_driver: fallback.configured_driver.clone(),
                    bound_driver: NULL_DRIVER_ID.to_string(),
                    reason: fallback.reason.clone(),
                },
            );
            bind_provider(
                Arc::new(NullMemoryProvider::new()),
                NULL_DRIVER_ID.to_string(),
                DriverClass::Null,
                cfg.hooks,
                // The fallback binds the in-process placeholder, so there is no
                // boundary to cross and nothing to trust-gate. The refused
                // driver's own trust_state is deliberately NOT carried over —
                // it describes a binding that did not happen.
                crate::openhuman::memory::guard::policy::TRUSTED.to_string(),
                Some(fallback),
            )
        }
    }
}

/// The single place `capabilities()` is asked. Every construction path — real
/// bind, fallback, and the test seam — goes through here, so the "asked once
/// per bind" property holds by construction rather than by convention.
fn bind_provider(
    provider: Arc<dyn MemoryProvider>,
    driver_id: String,
    class: DriverClass,
    hooks: MemoryHooksConfig,
    trust_state: String,
    fallback: Option<FallbackReason>,
) -> MemoryBinding {
    let capabilities = provider.capabilities();
    // Built on the same single path, so a binding can never exist without its
    // guard and no caller has to remember to construct one.
    let guard = Arc::new(MemoryGuard::new(
        Arc::clone(&provider),
        Arc::new(GuardPolicy::new(
            driver_id.clone(),
            class,
            hooks,
            trust_state,
        )),
    ));
    MemoryBinding {
        provider,
        guard,
        driver_id,
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
    bind_provider(
        provider,
        driver_id,
        class,
        MemoryHooksConfig::default(),
        crate::openhuman::memory::guard::policy::TRUSTED.to_string(),
        None,
    )
}

/// Per-workspace binding cache. Same shape as
/// `memory::people::store::STORES` — see the module docs for why this is a map
/// and not a slot.
///
/// Keyed on the **binding-relevant config as well as the path**, not the path
/// alone, because a config change for an already-bound workspace must produce a
/// fresh binding. `CoreContext::rebind_workspace` deliberately treats "same
/// workspace, changed `[subsystems.memory]`" as a real rebind — a changed
/// `driver` / `hooks` / `drivers` (trust) all feed `build`, so a path-only key
/// would keep serving the previous driver until restart. Carrying
/// `MemorySubsystemConfig` in the key (it derives `Hash`) means a changed
/// config hits a different slot and binds fresh, while a returned-to config
/// still resolves its original binding.
type BindingCacheKey = (PathBuf, MemorySubsystemConfig);
type BindingCache = RwLock<HashMap<BindingCacheKey, Arc<MemoryBinding>>>;

static BINDINGS: OnceLock<BindingCache> = OnceLock::new();

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
    let cache = BINDINGS.get_or_init(Default::default);
    let key = (workspace_dir.to_path_buf(), cfg.clone());
    if let Some(binding) = cache
        .read()
        .map_err(|e| format!("[memory:binding] cache read lock poisoned: {e}"))?
        .get(&key)
    {
        return Ok(Arc::clone(binding));
    }

    let binding = Arc::new(build(workspace_dir, cfg));

    let mut guard = cache
        .write()
        .map_err(|e| format!("[memory:binding] cache write lock poisoned: {e}"))?;
    // Re-check under the write lock: a racing caller may have bound the same
    // workspace (and config) while we were building. Reuse theirs so one
    // workspace never has two live drivers for the same config (kernel.md §3.1)
    // and `capabilities()` stays asked once.
    let entry = guard.entry(key).or_insert_with(|| Arc::clone(&binding));
    Ok(Arc::clone(entry))
}

#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
