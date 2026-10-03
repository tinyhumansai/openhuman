//! Core initialization context.
//!
//! [`CoreContext`] owns the core's initialization *order* (Phase 2, Stage A):
//! register controllers, load the master key, seed the RPC bearer, initialize
//! the workspace-bound stores, and run pure `bootstrap_core_runtime` registration. Today it is a
//! facade — the store init still targets the process globals — but centralizing
//! the sequence here is the seam the later stages build on (handler-threaded
//! context, per-context stores). See the pluggable-core work (`core::runtime`)
//! for how this fits with [`builder`](crate::core::runtime::builder).
//!
//! [`init_stores`] initializes the process-global stores bound to a single
//! resolved workspace directory (memory, image attachments, WhatsApp data,
//! people) plus the boot-time Sentry user binding. It preserves the exact
//! behavior and ordering of the original inline `run_server_inner` block,
//! including the deliberate wrong-workspace guard (never seed against a
//! `Config::default` fallback — Sentry OPENHUMAN-CORE-48 / TAURI-RUST-8NM).

use std::future::Future;
use std::sync::{Arc, OnceLock, RwLock};

use crate::core::runtime::TokenSource;
use crate::core::types::HostKind;

/// The process-wide default context — the first one built. Callers that dispatch
/// RPC without an explicit per-call context (the desktop shell, the CLI, tests)
/// resolve to this. Multi-tenant hosts override it per dispatch via
/// [`CoreContext::scope`].
static DEFAULT_CONTEXT: OnceLock<Arc<CoreContext>> = OnceLock::new();

tokio::task_local! {
    /// The context active for the current dispatch, set by [`CoreContext::scope`]
    /// at the `try_invoke_registered_rpc` chokepoint. Absent outside a scope —
    /// [`CoreContext::current`] then falls back to [`DEFAULT_CONTEXT`].
    static CURRENT_CONTEXT: Arc<CoreContext>;
}

/// A built, initialized core context. Holds the identity of the host and the
/// resolved workspace directory; created by [`CoreContext::init`].
///
/// Handlers reach the context for the current dispatch through
/// [`CoreContext::current`] rather than a threaded parameter — the ambient
/// context is established once per RPC at the dispatch chokepoint. This keeps
/// controller handlers as bare `fn` pointers (no per-handler signature churn)
/// while giving every handler a path to per-context state.
///
/// Stage A/B: state that today still lives in process globals is reached through
/// the globals as before; a domain migrates by reading its store handle off the
/// context ([`CoreContext::current`]) instead of the global. Once a domain's
/// state lives on the context, two contexts dispatched under distinct
/// [`CoreContext::scope`]s read isolated state — the Phase 3 exit criterion.
pub struct CoreContext {
    host_kind: HostKind,
    /// The workspace and its memory-driver configuration form one binding
    /// input. They must be read and updated together: a caller that observes a
    /// new workspace with the previous user's memory config could cache a
    /// permanently incorrect memory binding for that workspace.
    /// Shared (`Arc`) so a context derived without a memory override follows
    /// the parent when the memory engine is switched.
    workspace_binding: RwLock<Arc<RwLock<WorkspaceBinding>>>,
    /// Which domain families are live for this context (#4796). The registry
    /// filters its controller/schema/dispatch surface by this set via
    /// [`CoreContext::current`] → [`CoreContext::domains`]. `full()` for the
    /// desktop shell / standalone CLI (byte-identical to pre-#4796).
    domains: crate::core::runtime::DomainSet,
    /// The configuration an embedder supplied to
    /// [`CoreBuilder::config`](crate::core::runtime::CoreBuilder::config),
    /// if any.
    ///
    /// `None` for every host that lets the core discover its own config, which
    /// is all of them today except a library embedder — so the default path is
    /// untouched.
    ///
    /// This exists because setting the config at boot is **not** sufficient on
    /// its own: RPC handlers do not receive it, they call
    /// `config::ops::load_config_with_timeout()` per dispatch, which re-runs
    /// `Config::load_or_init()` and re-resolves the process-global workspace.
    /// An embedder that supplied a config would therefore watch its turns run
    /// against `~/.openhuman` anyway. Publishing it on the context — the seam
    /// phase 2 of the pluggable-core work (`core::runtime`) introduced for exactly this
    /// migration — lets that loader prefer it without any handler changing.
    embedder_config: Option<crate::config::Config>,
    /// Per-tool-group disclosure for this context (see
    /// [`ToolGroups`](crate::tools::toolpacks::ToolGroups)).
    ///
    /// The third narrowing axis, independent of `domains` the same way
    /// `DomainSet` is independent of `ServiceSet`: `DomainSet` decides which
    /// families *exist*, `ToolGroups` decides how the ones that exist reach
    /// the model. Defaults to every group withheld, which is what the
    /// compiled-in pack table meant before the type existed.
    tool_groups: crate::tools::toolpacks::ToolGroups,
    /// Whether skill discovery under this context scans the operator's
    /// user-scope roots (`~/.openhuman/skills`, `~/.agents/skills`).
    ///
    /// `true` for every booted context — the desktop, CLI and a plain library
    /// embedder all run as the operator. A per-agent context derived through
    /// [`CoreContext::derive_with`] can turn it off so an embedded agent sees
    /// only the skills its host installed for it, never the operator's.
    user_skill_roots: bool,
    /// The backend transport this context's handlers reach the hosted
    /// TinyHumans backend through, when the host supplied one via
    /// [`CoreBuilder::backend_transport`](crate::core::runtime::CoreBuilder::backend_transport).
    /// `None` means "fall back to the process-global transport" — see
    /// [`crate::backend::transport::resolve_backend_transport`] for the order.
    /// Inherited unchanged by every context derived through
    /// [`CoreContext::derive_with`].
    backend_transport: Option<Arc<dyn crate::backend::transport::BackendTransport>>,
}

/// Per-agent overrides layered onto a booted context by
/// [`CoreContext::derive_with`].
///
/// This is the seam a library host uses to run many independently configured
/// agents on one booted core: each agent gets its own `Config` (provider
/// routes, MCP servers, autonomy tier, `action_dir`), its own
/// [`DomainSet`](crate::core::runtime::DomainSet), its own
/// [`ToolGroups`](crate::tools::toolpacks::ToolGroups) and its own skill-root
/// policy, while sharing the host identity, keyring, bus and RPC bearer of the
/// context it derives from.
#[derive(Debug, Clone)]
pub struct ContextOverlay {
    /// The config every handler dispatched under the derived context reads
    /// through `config::ops::load_config_with_timeout()`. Keep `config_path`
    /// equal to the parent's: credentials, auth profiles and the keyring file
    /// backend all resolve against its parent directory.
    pub config: crate::config::Config,
    /// Domain families live for the derived context. Only narrowing the parent
    /// is meaningful: controllers a booted core never registered stay absent
    /// no matter what this says.
    pub domains: crate::core::runtime::DomainSet,
    /// Tool-group disclosure for the derived context.
    pub tool_groups: crate::tools::toolpacks::ToolGroups,
    /// Scan the operator's user-scope skill roots (`true` = today's behaviour).
    pub user_skill_roots: bool,
}

impl ContextOverlay {
    /// An overlay that keeps user-scope skill roots visible.
    pub fn new(
        config: crate::config::Config,
        domains: crate::core::runtime::DomainSet,
        tool_groups: crate::tools::toolpacks::ToolGroups,
    ) -> Self {
        Self {
            config,
            domains,
            tool_groups,
            user_skill_roots: true,
        }
    }

    /// Hide the operator's `~/.openhuman/skills` / `~/.agents/skills` from
    /// skill discovery under the derived context.
    pub fn without_user_skill_roots(mut self) -> Self {
        self.user_skill_roots = false;
        self
    }
}

/// The complete input to a workspace-scoped memory binding.
///
/// This is deliberately one value behind one lock. `MemoryBinding` caches by
/// this pair, so splitting either its read or update would let concurrent RPC
/// traffic associate a workspace with another user's driver, hooks, or trust
/// policy. The config is captured at build time so
/// [`CoreContext::memory_binding`] stays synchronous and I/O-free.
struct WorkspaceBinding {
    workspace_dir: Option<std::path::PathBuf>,
    memory_subsystem: crate::config::schema::MemorySubsystemConfig,
}

/// Say so when the workspace is rebound after the memory module has already
/// loaded.
///
/// The module is handed its `config_path` once, when it loads, and tinybus
/// never unloads a library — there is no shutdown path and nothing a shutdown
/// could reclaim (`modules/host.rs`). So a rebind after the module is `Ready`
/// leaves it reading the *previous* profile's source registry for the rest of
/// the process: the host writes `[[memory_sources]]` into the new profile's
/// `config.toml`, and every id it registers is unknown to the driver.
///
/// This is the boot-signed-out-then-log-in case. It cannot be repaired in
/// process, so this does not try. It makes the moment the binding went stale
/// visible in the log, beside the rebind that caused it, instead of leaving a
/// bare `NotFound` on a sync minutes later as the only evidence.
///
/// Best-effort and never fatal: a build without the modules feature, or a
/// process whose memory module never loaded, has nothing stale to report.
#[cfg(feature = "modules")]
fn warn_if_memory_module_outlived_its_profile(workspace_dir: &std::path::Path) {
    use crate::modules::types::ModuleState;
    if crate::modules::state_of(crate::modules::memory::MODULE_ID) == ModuleState::Ready {
        log::warn!(
            "[core-context] workspace rebound to {} while the memory module is already loaded. \
             The module keeps the source registry it was given when it loaded and cannot be \
             rebound in this process, so memory sources registered under this profile will not \
             be visible to it until the app is restarted.",
            workspace_dir.display()
        );
    }
}

#[cfg(not(feature = "modules"))]
fn warn_if_memory_module_outlived_its_profile(_workspace_dir: &std::path::Path) {}

impl CoreContext {
    /// Run the core initialization sequence and return the context plus whether
    /// an operator-supplied RPC bearer exists (for the public-bind safety check
    /// in `openhuman_rpc::server::serve`) plus the loaded config, when boot reached
    /// workspace-bound init. Order is load-bearing and mirrors the original
    /// `run_server_inner` sequence:
    ///
    /// 1. register controllers, 2. master key, 3. seed RPC bearer,
    /// 4. workspace stores ([`init_stores`]), 5. pure runtime registration.
    ///
    /// `preloaded_config` lets an embedder supply the [`Config`] outright
    /// instead of having step 4 discover one from disk and the environment. See
    /// [`init_with_config`](Self::init_with_config) for why that matters.
    pub async fn init(
        host_kind: HostKind,
        token: &TokenSource,
        domains: crate::core::runtime::DomainSet,
    ) -> anyhow::Result<(Arc<CoreContext>, bool, Option<crate::config::Config>)> {
        Self::init_with_config(host_kind, token, domains, Default::default(), None, None).await
    }

    /// [`init`](Self::init) with an optional caller-supplied configuration.
    ///
    /// Passing `Some(config)` skips `Config::load_or_init()` entirely — the
    /// config is used verbatim, exactly as loaded config would be. This is the
    /// seam that lets a library embedder configure the core with struct fields
    /// rather than by mutating the process environment before `build()`, which
    /// is order-dependent, process-global, and invisible at the call site.
    ///
    /// Note it does not make the core hermetic on its own: `init_stores`, the
    /// session database and the keyring still write beneath
    /// `config.workspace_dir`. It decides *where*, not *whether*.
    pub async fn init_with_config(
        host_kind: HostKind,
        token: &TokenSource,
        domains: crate::core::runtime::DomainSet,
        tool_groups: crate::tools::toolpacks::ToolGroups,
        preloaded_config: Option<crate::config::Config>,
        backend_transport: Option<Arc<dyn crate::backend::transport::BackendTransport>>,
    ) -> anyhow::Result<(Arc<CoreContext>, bool, Option<crate::config::Config>)> {
        log::debug!(
            "[core-context] init: host_kind={host_kind:?} domains={domains:?} \
             tool_groups={tool_groups:?} backend_transport={}",
            backend_transport
                .as_ref()
                .map(|t| t.name())
                .unwrap_or("<process-global>")
        );
        // 1. Ensure all controllers are registered before anything dispatches.
        let _ = crate::core::all::all_registered_controllers();

        // 2. Load the master encryption key before any config/credential op that
        //    needs to decrypt secrets. No-op if already called (e.g. from
        //    run_core_from_args for the CLI).
        crate::security::keyring::init_master_key().map_err(anyhow::Error::msg)?;

        // 4. Seed the per-process RPC bearer. `Fixed` seeds the in-memory value
        //    directly (never touches the env); `EnvOrFile` reads
        //    OPENHUMAN_CORE_TOKEN or generates + writes {root}/core.token.
        //
        //    `has_operator_token` records whether an OPERATOR-supplied bearer
        //    exists (in-memory handoff or env var). The self-generated core.token
        //    file does NOT count — remote clients cannot read it — so it must not
        //    satisfy the public-bind safety check in `serve`.
        let has_operator_token = match token {
            TokenSource::Fixed(token) => {
                crate::core::auth::init_rpc_token_with_value(token)?;
                !token.trim().is_empty()
            }
            TokenSource::EnvOrFile => {
                // A caller-supplied config scopes the core's state, so a
                // self-generated bearer must land beside it rather than under
                // the operator's real `~/.openhuman` root — otherwise an
                // "ephemeral" harness still writes a `core.token` into the
                // operator's install. Fall back to the default root only when
                // no config was supplied.
                let token_dir = preloaded_config
                    .as_ref()
                    .map(|cfg| {
                        cfg.config_path
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| cfg.config_path.clone())
                    })
                    .unwrap_or_else(|| {
                        crate::config::default_root_openhuman_dir().unwrap_or_else(|_| {
                            dirs::home_dir()
                                .unwrap_or_else(|| std::path::PathBuf::from("."))
                                .join(".openhuman")
                        })
                    });
                crate::core::auth::init_rpc_token(&token_dir)?;
                std::env::var(crate::core::auth::CORE_TOKEN_ENV_VAR)
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .is_some()
            }
        };

        // 5. Resolve config once, then initialize workspace-bound stores
        //    (memory, attachments, people) with that exact workspace.
        // Kept for the context: `preloaded_config` is consumed below, and the
        // whole point is that handlers can reach it after boot.
        let embedder_config = preloaded_config.clone();
        let loaded = match preloaded_config {
            // A supplied config is authoritative: no disk read, no env overlay,
            // and no `Err` arm to reach, because there was nothing to fail.
            Some(cfg) => {
                log::debug!("[core-context] init: using caller-supplied config (scoped workspace)");
                Ok(cfg)
            }
            None => crate::config::Config::load_or_init().await,
        };
        let config = match loaded {
            Ok(cfg) => {
                init_stores(&cfg, domains).await;
                Some(cfg)
            }
            Err(e) => {
                log::error!(
                    "[boot] workspace-bound store init SKIPPED — \
                     Config::load_or_init failed ({e:#}). Memory persistence is \
                     DISABLED for this run; no silent fallback to the default \
                     workspace (which would cause chunk loss / cross-workspace \
                     bleed-over). Fix config.toml or set OPENHUMAN_WORKSPACE to a \
                     writable path, then restart."
                );
                None
            }
        };
        let workspace_dir = config.as_ref().map(|cfg| cfg.workspace_dir.clone());

        // 6. Long-lived runtime infrastructure: event bus, domain subscribers,
        //    ledgers, agent-definition registry, live security policy, approval
        //    gate, socket manager. Idempotent (Once-guarded internally). Selected
        //    background jobs start later, from CoreRuntime::start_services(), after a transport binds
        //    succeeds.
        let runtime_config = config.clone();
        let memory_subsystem = config
            .as_ref()
            .map(|cfg| cfg.subsystems.memory.clone())
            .unwrap_or_default();
        super::bootstrap::bootstrap_core_runtime(host_kind, config, domains).await;

        let ctx = Arc::new(CoreContext {
            host_kind,
            workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                workspace_dir,
                memory_subsystem,
            }))),
            domains,
            tool_groups,
            embedder_config,
            user_skill_roots: true,
            backend_transport,
        });

        // Register the process default context (first build wins). Dispatch
        // resolves to this when no per-call context is scoped.
        let _ = DEFAULT_CONTEXT.set(ctx.clone());

        Ok((ctx, has_operator_token, runtime_config))
    }

    /// The host that constructed this context (Tauri shell / CLI / Docker).
    pub fn host_kind(&self) -> HostKind {
        self.host_kind
    }

    /// Which domain families are live for this context (#4796). The controller
    /// registry consults this (via [`CoreContext::current`]) to filter its
    /// schema/dispatch/tool surface. `full()` for desktop/CLI.
    /// Per-group tool disclosure for this context.
    pub fn tool_groups(&self) -> crate::tools::toolpacks::ToolGroups {
        self.tool_groups.clone()
    }

    pub fn domains(&self) -> crate::core::runtime::DomainSet {
        self.domains
    }

    /// Whether skill discovery under this context scans the operator's
    /// user-scope roots. See [`ContextOverlay::user_skill_roots`].
    pub fn user_skill_roots(&self) -> bool {
        self.user_skill_roots
    }

    /// [`user_skill_roots`](Self::user_skill_roots) of the ambient context, or
    /// `true` when nothing is scoped and no default context exists — the
    /// pre-existing behaviour for every non-embedded caller.
    pub fn current_user_skill_roots() -> bool {
        Self::current().is_none_or(|ctx| ctx.user_skill_roots())
    }

    /// A child context sharing this one's host identity, with its own config,
    /// domain set, tool groups and skill-root policy.
    ///
    /// **No boot runs.** No stores are initialised, no registry, gate or live
    /// policy is (re)installed, and the process default context is untouched.
    /// The child is only useful inside [`CoreContext::scope`] (or
    /// `CoreRuntime::invoke_in` / `run_in`), where every reader that goes
    /// through [`CoreContext::current`] — the config loader, the DomainSet
    /// dispatch gate, the tool-group filter, skill discovery — sees the
    /// overlay instead of the parent.
    ///
    /// The workspace binding is anchored to `overlay.config.workspace_dir`, so
    /// an agent with its own workspace subdirectory resolves its own memory
    /// binding lazily, exactly as an embedder-supplied config does at boot.
    pub fn derive_with(&self, overlay: ContextOverlay) -> Arc<CoreContext> {
        // Clamped to what this context can already dispatch. A derived
        // overlay may only narrow: callers outside this crate (embed's
        // `Runtime::agent`, for one) already refuse a spec that names a
        // family the runtime never registered, but that check lives at the
        // call site. Enforcing it here too means a future or third-party
        // caller of `derive_with` cannot widen a restricted parent's domains
        // just by omitting that check.
        let domains = self.domains.intersect(&overlay.domains);
        if domains != overlay.domains {
            log::warn!(
                "[core-context] derive: overlay requested domains={:?} wider than \
                 parent domains={:?}; clamped to {:?}",
                overlay.domains,
                self.domains,
                domains
            );
        }
        log::debug!(
            "[core-context] derive: workspace_dir={} domains={:?} tool_groups={:?} \
             user_skill_roots={}",
            overlay.config.workspace_dir.display(),
            domains,
            overlay.tool_groups,
            overlay.user_skill_roots
        );
        // A derived context that keeps the parent's workspace and memory config
        // shares the parent's binding handle, so an engine switch reaches it. One
        // that deliberately carries its own (another workspace, or its own
        // `[subsystems.memory]`) keeps that override.
        let shared_binding = {
            let parent_handle = self.workspace_binding.read();
            let parent_handle = parent_handle
                .as_ref()
                .ok()
                .map(|handle| Arc::clone(&**handle));
            let can_share = parent_handle.as_ref().is_some_and(|handle| {
                handle.read().ok().is_some_and(|parent| {
                    parent.workspace_dir.as_deref() == Some(overlay.config.workspace_dir.as_path())
                        && (parent.memory_subsystem == overlay.config.subsystems.memory
                            || self.embedder_config.as_ref().is_some_and(|config| {
                                config.workspace_dir == overlay.config.workspace_dir
                                    && config.subsystems.memory == overlay.config.subsystems.memory
                            }))
                })
            });
            match (can_share, parent_handle.as_ref()) {
                (true, Some(handle)) => Arc::clone(handle),
                _ => Arc::new(RwLock::new(WorkspaceBinding {
                    workspace_dir: Some(overlay.config.workspace_dir.clone()),
                    memory_subsystem: overlay.config.subsystems.memory.clone(),
                })),
            }
        };
        Arc::new(CoreContext {
            host_kind: self.host_kind,
            workspace_binding: RwLock::new(shared_binding),
            domains,
            tool_groups: overlay.tool_groups,
            embedder_config: Some(overlay.config),
            user_skill_roots: overlay.user_skill_roots,
            backend_transport: self.backend_transport.clone(),
        })
    }

    /// The backend transport bound to this context, if the host supplied one.
    pub fn backend_transport(
        &self,
    ) -> Option<Arc<dyn crate::backend::transport::BackendTransport>> {
        self.backend_transport.clone()
    }

    /// The resolved per-user workspace directory this context is bound to.
    pub fn workspace_dir(&self) -> Result<std::path::PathBuf, String> {
        self.workspace_binding
            .read()
            .map_err(|e| format!("workspace unavailable: context lock poisoned: {e}"))?
            .read()
            .map_err(|e| format!("workspace unavailable: binding lock poisoned: {e}"))?
            .workspace_dir
            .clone()
            .ok_or_else(|| {
                "workspace unavailable: Config::load_or_init failed during core boot; \
                 fix config.toml or OPENHUMAN_WORKSPACE and restart"
                    .to_string()
            })
    }

    /// The bound memory driver for this context's workspace — the memory
    /// subsystem's binding seam (`docs/specs/kernel.md` §3.1). Deliberately the
    /// same shape as [`CoreContext::people`]: two contexts over different
    /// workspaces get isolated bindings, one context always gets the same
    /// cached binding, and an active-user switch that goes through
    /// [`CoreContext::rebind_default_workspace`] automatically resolves the
    /// new workspace's binding — including its `[subsystems.memory]` config,
    /// which the rebind carries along with the workspace dir.
    ///
    /// That last property is why there is **no** explicit "rebind the memory
    /// driver" call at the login / logout / revalidation sites the way
    /// `memory::global::init` needs one: the accessor keys on the workspace
    /// dir and the subsystem config, both of which those sites already re-point.
    ///
    /// It also structurally supersedes `memory::global`'s
    /// clear-on-failed-rebind guard. There is no shared slot that could keep
    /// pointing at the previous workspace, so a failed bind for workspace B
    /// cannot hand back workspace A's driver. Pinned by
    /// `failed_bind_never_returns_previous_workspace_binding`.
    pub fn memory_binding(&self) -> Result<Arc<crate::memory::binding::MemoryBinding>, String> {
        let binding_handle = self
            .workspace_binding
            .read()
            .map_err(|e| format!("[core-context] binding handle lock poisoned: {e}"))?;
        let binding = binding_handle
            .read()
            .map_err(|e| format!("[core-context] workspace binding lock poisoned: {e}"))?;
        let workspace_dir = binding.workspace_dir.clone();
        let memory_subsystem = binding.memory_subsystem.clone();
        drop(binding);
        let workspace_dir = workspace_dir.ok_or_else(|| {
            "workspace unavailable: Config::load_or_init failed during core boot; \
             fix config.toml or OPENHUMAN_WORKSPACE and restart"
                .to_string()
        })?;
        crate::memory::binding::for_workspace(&workspace_dir, &memory_subsystem)
    }

    /// The bound driver's advertised capability set. Cheap (a `Copy` bitset
    /// read off the cached binding), infallible, and **OPEN by default**: when
    /// no workspace is bound, or the binding cannot be resolved, this returns
    /// the full set.
    ///
    /// That default mirrors `core::all::group_allowed`, which returns `true`
    /// with no ambient context. Roughly 4000 unit tests run pre-boot with no
    /// bound driver; a deny-by-default here would turn every memory test red at
    /// once. Denying is only ever correct once a driver has actually answered
    /// `capabilities()`.
    ///
    /// One case answers **closed**: a deliberate `[subsystems.memory] driver =
    /// "null"` returns the empty set, not the null driver's mandatory three.
    /// The driver honestly advertises those three — `subsystems_status` still
    /// reports them — but an operator who bound `/dev/null` asked for the whole
    /// memory surface to be gone, and leaving the mandatory families registered
    /// would keep `memory_store` / `memory_recall` / `memory.list_documents`
    /// answering off the embedded store the guarded re-point has not yet
    /// covered. See [`MemoryBinding::disables_memory`](crate::memory::binding::MemoryBinding::disables_memory).
    pub fn memory_capabilities(&self) -> tinymemory_api::capabilities::Capabilities {
        self.memory_binding()
            .map(|binding| {
                if binding.disables_memory() {
                    tinymemory_api::capabilities::Capabilities::default()
                } else {
                    binding.capabilities()
                }
            })
            .unwrap_or_else(|_| crate::memory::binding::unbound_default_capabilities())
    }

    /// The **guarded** memory driver for this context's workspace — the handle
    /// product code should hold (`docs/specs/kernel.md` §3.4).
    ///
    /// The guard implements the same `MemoryProvider` contract as the driver it
    /// wraps, so it is a drop-in for a caller that already speaks the contract,
    /// and its family accessors hand back guarded handles rather than the raw
    /// driver's — which is what makes the policy unskippable for anyone holding
    /// it.
    ///
    /// [`Self::memory_binding`] still exists and still exposes the bare
    /// provider. That is deliberate and narrow: the one production caller is
    /// the health probe in `memory::ops::provider`, and a liveness probe is not
    /// product code — routing it through the guard would let an autonomy tier
    /// break status output. New call sites use this accessor.
    ///
    /// # Errors
    ///
    /// As [`Self::memory_binding`]: only when the workspace dir cannot be
    /// resolved or the binding cache lock is poisoned.
    pub fn memory(&self) -> Result<Arc<crate::memory::guard::MemoryGuard>, String> {
        Ok(self.memory_binding()?.guard())
    }

    /// The capability set for the current dispatch, or the open default when
    /// there is no context at all. This is the direct analogue of
    /// `core::all::group_allowed` and is the function a future capability
    /// registration filter calls.
    pub fn current_memory_capabilities() -> tinymemory_api::capabilities::Capabilities {
        Self::current()
            .map(|ctx| ctx.memory_capabilities())
            .unwrap_or_else(crate::memory::binding::unbound_default_capabilities)
    }

    /// The context for the current dispatch: the one scoped by
    /// [`CoreContext::scope`] if inside a scope, else the process
    /// [`DEFAULT_CONTEXT`]. Returns `None` only before any context is built
    /// (e.g. a unit test that dispatches without initializing the core).
    ///
    /// Handlers migrating off process globals read their state through this.
    /// The configuration this context was built with, when an embedder
    /// supplied one.
    ///
    /// `None` means "discover it the usual way" — see the field docs.
    pub fn embedder_config(&self) -> Option<&crate::config::Config> {
        self.embedder_config.as_ref()
    }

    /// The embedder-supplied config for the current dispatch, if there is one.
    ///
    /// The read path for `config::ops::load_config_with_timeout`.
    pub fn current_embedder_config() -> Option<crate::config::Config> {
        Self::current().and_then(|ctx| ctx.embedder_config.clone())
    }

    pub fn current() -> Option<Arc<CoreContext>> {
        CURRENT_CONTEXT
            .try_with(|ctx| ctx.clone())
            .ok()
            .or_else(|| DEFAULT_CONTEXT.get().cloned())
    }

    /// The process default context (first built), independent of any active
    /// scope. Used by the dispatch chokepoint to establish the ambient scope.
    pub fn default_context() -> Option<Arc<CoreContext>> {
        DEFAULT_CONTEXT.get().cloned()
    }

    /// Rebind the process default context to the current active user's
    /// workspace **and** that user's `[subsystems.memory]` config. Desktop
    /// login, logout, and pending-session revalidation can switch the active
    /// workspace after boot without rebuilding the core; every call site
    /// already holds the target `Config`, so passing the config here keeps the
    /// bound driver (and its hooks / trust settings) from silently carrying
    /// over from the previous user. Scoped multi-tenant dispatch is unaffected
    /// because tenant contexts are passed to [`CoreContext::scope`] explicitly
    /// and are not the process default.
    pub fn rebind_default_workspace(
        workspace_dir: &std::path::Path,
        memory_subsystem: crate::config::schema::MemorySubsystemConfig,
    ) -> Result<(), String> {
        let Some(ctx) = DEFAULT_CONTEXT.get() else {
            log::debug!(
                "[core-context] default context not initialized; skipped workspace rebind to {}",
                workspace_dir.display()
            );
            return Ok(());
        };
        ctx.rebind_workspace(workspace_dir, memory_subsystem)
    }

    /// Replace only the `[subsystems.memory]` half of this context's workspace
    /// binding, keeping the workspace dir. The memory-engine switch
    /// (`memory::binding::rebind`) uses it so [`Self::memory_binding`] resolves
    /// the new driver without a restart. A no-op when the config is unchanged.
    pub(crate) fn set_memory_subsystem(
        &self,
        workspace_dir: &std::path::Path,
        memory_subsystem: crate::config::schema::MemorySubsystemConfig,
    ) -> Result<(), String> {
        let binding_handle = self.workspace_binding.write().map_err(|e| {
            format!("memory subsystem update failed: binding handle lock poisoned: {e}")
        })?;
        let mut binding = binding_handle
            .write()
            .map_err(|e| format!("memory subsystem update failed: binding lock poisoned: {e}"))?;
        if binding.workspace_dir.as_deref() != Some(workspace_dir)
            || binding.memory_subsystem == memory_subsystem
        {
            return Ok(());
        }
        log::info!(
            "[core-context] memory subsystem driver set to '{}' for {}",
            memory_subsystem.driver,
            workspace_dir.display()
        );
        binding.memory_subsystem = memory_subsystem;
        Ok(())
    }

    fn rebind_workspace(
        &self,
        workspace_dir: &std::path::Path,
        memory_subsystem: crate::config::schema::MemorySubsystemConfig,
    ) -> Result<(), String> {
        let mut binding_handle = self
            .workspace_binding
            .write()
            .map_err(|e| format!("workspace rebind failed: binding handle lock poisoned: {e}"))?;
        let binding = binding_handle
            .read()
            .map_err(|e| format!("workspace rebind failed: binding lock poisoned: {e}"))?;
        if binding.workspace_dir.as_deref() == Some(workspace_dir) {
            if binding.memory_subsystem != memory_subsystem {
                log::info!(
                    "[core-context] rebound memory subsystem for {} to driver='{}'",
                    workspace_dir.display(),
                    memory_subsystem.driver
                );
                drop(binding);
                binding_handle
                    .write()
                    .map_err(|e| format!("workspace rebind failed: binding lock poisoned: {e}"))?
                    .memory_subsystem = memory_subsystem;
            } else {
                log::debug!(
                    "[core-context] workspace {} already bound with the current subsystem config",
                    workspace_dir.display()
                );
            }
            return Ok(());
        }
        log::info!(
            "[core-context] rebound default workspace to {} with memory subsystem driver='{}'",
            workspace_dir.display(),
            memory_subsystem.driver
        );
        drop(binding);
        *binding_handle = Arc::new(RwLock::new(WorkspaceBinding {
            workspace_dir: Some(workspace_dir.to_path_buf()),
            memory_subsystem,
        }));
        warn_if_memory_module_outlived_its_profile(workspace_dir);
        Ok(())
    }

    /// Run `fut` with `ctx` as the ambient [`CoreContext::current`]. The dispatch
    /// layer wraps each handler invocation in this; multi-tenant hosts pass the
    /// tenant's context here so the handler's `current()` reads isolated state.
    pub async fn scope<F: Future>(ctx: Arc<CoreContext>, fut: F) -> F::Output {
        CURRENT_CONTEXT.scope(ctx, fut).await
    }

    /// Capture the current context now and carry it across a subsequently
    /// spawned task. Calling this before `tokio::spawn` is essential: reading
    /// `current()` inside the child would already have fallen back to the
    /// process default.
    pub fn propagate<F: Future>(fut: F) -> impl Future<Output = F::Output> {
        let ctx = Self::current();
        async move {
            match ctx {
                Some(ctx) => Self::scope(ctx, fut).await,
                None => fut.await,
            }
        }
    }

    /// Test-only constructor: build a context with an explicit
    /// [`DomainSet`](crate::core::runtime::DomainSet) and optional workspace, so
    /// cross-module tests (e.g. `core::all`'s registry filter) can exercise the
    /// ambient DomainSet gate without going through the full [`CoreContext::init`]
    /// boot sequence.
    ///
    /// `memory_subsystem` is the seam the capability tests need: pass `None`
    /// for the default (`driver = "tinycortex"`, no driver table), or an
    /// explicit config to exercise the fallback / trust paths without a boot.
    /// It takes the *config* rather than a `Capabilities` value on purpose —
    /// injecting a capability set directly would let a test assert a set no
    /// driver could have advertised, bypassing the very `admit` +
    /// `capabilities()` path that has to be proven.
    #[cfg(test)]
    pub(crate) fn for_test(
        domains: crate::core::runtime::DomainSet,
        workspace_dir: Option<std::path::PathBuf>,
        memory_subsystem: Option<crate::config::schema::MemorySubsystemConfig>,
    ) -> Arc<CoreContext> {
        Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                workspace_dir,
                memory_subsystem: memory_subsystem.unwrap_or_default(),
            }))),
            domains,
            tool_groups: Default::default(),
            embedder_config: None,
            user_skill_roots: true,
            backend_transport: None,
        })
    }

    /// Test-only constructor that carries an embedder-supplied config, so a
    /// cross-module test can exercise the `load_config_with_timeout()` read
    /// path (which prefers [`CoreContext::current_embedder_config`]) without a
    /// full boot or a racy on-disk `config.toml`.
    ///
    /// Distinct from [`CoreContext::for_test`] — which always sets
    /// `embedder_config: None` — so the ~30 existing `for_test` call sites are
    /// unaffected. The workspace binding is anchored to the config's
    /// `workspace_dir`, matching how [`CoreContext::init`] wires an
    /// embedder-supplied config.
    #[cfg(test)]
    pub(crate) fn for_test_with_config(
        domains: crate::core::runtime::DomainSet,
        config: crate::config::Config,
    ) -> Arc<CoreContext> {
        Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                workspace_dir: Some(config.workspace_dir.clone()),
                memory_subsystem: Default::default(),
            }))),
            domains,
            tool_groups: Default::default(),
            embedder_config: Some(config),
            user_skill_roots: true,
            backend_transport: None,
        })
    }
}

/// Bind the memory driver for this workspace and initialize the other
/// workspace-bound stores.
///
/// This no longer initializes an in-process `MemoryClient`: the memory
/// subsystem is reached through [`crate::memory::binding`], which is
/// a workspace-keyed cache rather than a process-global slot (#5560). The
/// engine handle that `memory::global` still hands out is a lazy singleton, so
/// the remaining holders construct it on first use.
///
/// A `Config::load_or_init` failure here is operator-visible and serious
/// (corrupt toml, bad permissions, missing/unwritable `OPENHUMAN_WORKSPACE` —
/// common on headless/containerised deploys with no writable `$HOME`).
/// Previously the fallback to `Config::default()` initialised the memory
/// store against the *wrong* workspace dir, silently causing
/// chunk loss / cross-workspace bleed-over while the app looked healthy (Sentry
/// OPENHUMAN-CORE-48). Instead: skip the workspace-bound init entirely so
/// memory stays explicitly *uninitialised* — callers then get a clear "memory
/// client not ready" error rather than reading/writing the wrong workspace. The
/// server still comes up; the operator sees the loud error and fixes their
/// config or sets `OPENHUMAN_WORKSPACE` to a writable path, then restarts.
/// Per-`DomainGroup` gating decision for each workspace-bound store that
/// [`init_stores`] initializes. Extracted as a pure value so the store-gating
/// mapping (which store is owned by which `DomainGroup`) has a single source of
/// truth that `init_stores` consumes and tests assert directly — without
/// touching process-global store state or booting a runtime (#4796 DoD item 3).
///
/// The keyring-path log and the credentials Sentry bind in `init_stores` are
/// intentionally *not* represented here: they are unguarded core infra every
/// `DomainSet` needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreInitPlan {
    /// The memory driver binding (`memory::binding`) — gated on
    /// [`DomainGroup::Memory`].
    pub memory: bool,
    /// `agent::multimodal` attachments sidecar dir — gated on [`DomainGroup::Agent`].
    pub agent_attachments: bool,
    /// legacy-workflow prune under `skills::registry` — gated on [`DomainGroup::Skills`].
    pub skills_prune: bool,
}

impl StoreInitPlan {
    /// The store-init plan for `domains`. Pure: no side effects, no globals.
    pub fn for_domains(domains: crate::core::runtime::DomainSet) -> Self {
        use crate::core::all::DomainGroup;
        Self {
            memory: domains.allows(DomainGroup::Memory),
            agent_attachments: domains.allows(DomainGroup::Agent),
            skills_prune: domains.allows(DomainGroup::Skills),
        }
    }
}

pub async fn init_stores(cfg: &crate::config::Config, domains: crate::core::runtime::DomainSet) {
    let plan = StoreInitPlan::for_domains(domains);

    let keyring_dir = crate::security::keyring::store::workspace_dir_for_file_backend();
    // Keyring path log + credentials Sentry bind (below) are unguarded — they
    // are core infra every DomainSet needs. Each workspace-bound store init is
    // gated on its owning DomainGroup so an excluded domain's store stays
    // uninitialized under `harness()`/`none()` (#4796 DoD item 3).
    log::info!(
        "[boot] paths: config={} workspace={} keyring_dir={} keyring_backend={} domains={:?}",
        cfg.config_path.display(),
        cfg.workspace_dir.display(),
        keyring_dir.display(),
        crate::security::keyring::backend_name(),
        domains,
    );
    if plan.memory {
        // The engine seams are gone from here (#5560). They installed embedding
        // / chat / config / NLP / scheduler / shutdown / error-reporting
        // callbacks into *this process's* copy of `tinymemory-core`, and that
        // copy no longer exists: the crate has left `[dependencies]`, and with
        // openhuman#6161 it has left `[dev-dependencies]` too, taking
        // `memory::host_impls` and the `memory-engine-seams` feature that
        // gated it. The module answers these over the bus through
        // `modules::memory_host` instead.
        //
        // The first attempt at this removal shipped an outage, and the reason
        // is worth keeping. It was not that the seams were needed in the
        // abstract — it was that `session::builder::factory` still reached
        // `store::factories::create_session_memory_with_local_ai`, which calls
        // `require_embedding_host()` on the chat hot path, so every chat turn
        // died with "no EmbeddingHost installed". That caller is gone, along
        // with `ops::helpers::active_memory_client`, the `global::{init,
        // client_if_ready}` sites and the `tree_runtime` glob; the only
        // remaining namers of the engine crate are test-only, served by
        // the `[dev-dependencies]` entry. A dev-dependency is not linked into
        // the shipped binary, so there is nothing left here to call back.
        //
        // The event sink is NOT one of those seams and must stay. It installs
        // into `tinymemory-api` — the contract crate, still a normal
        // dependency — and `memory::sync::composio::bus` publishes
        // `ComposioIntegrationsChanged` through it from production host code.
        // `tinymemory_api::events::publish` *silently drops* when unwired, by
        // design, so losing this install would be an invisible regression
        // rather than a loud one.
        crate::memory::host::install_memory_event_sink();
        // Publish the config a module-backed memory driver should load
        // against, before the binding below can construct one. Boot-only and
        // idempotent (first call wins) — see `modules::memory::set_modules_policy`
        // for why this must be a process-global rather than threaded through
        // `MemoryBinding::for_workspace`.
        #[cfg(feature = "modules")]
        crate::modules::memory::set_modules_policy(Arc::new(cfg.clone()));
        // ── No second engine is booted here any more (#5560 phase F) ────────
        //
        // This block used to call `tinymemory_core::global::init(...)` directly
        // above the bind below, so boot left **two** live `MemoryClient`s over
        // one `<workspace>/memory/memory.db`: the loadable TinyMemory module
        // reached over TinyBus, and a second in-process copy of the engine
        // crate. `memory::binding`'s module docs and
        // `CoreContext::memory_binding`'s both already argued that the
        // workspace-keyed binding map supersedes that process-global slot —
        // the slot needs a clear-on-failed-rebind guard, the map structurally
        // cannot hand workspace B's caller workspace A's driver — and this is
        // where that argument is executed.
        //
        // `memory::global` is a lazy singleton, so the callers that still hold
        // an in-process handle (`memory::ops::helpers::active_memory_client`,
        // `agent::experience::ops`, the session builder's shared-experience
        // handle, `openhuman memory ingest`/`query`) construct it on first use
        // exactly as before. What changes is that a boot which never reaches
        // one no longer pays for it — and that the engine's own lifetime is now
        // owned by the code that still needs it rather than by kernel boot.
        //
        // Bind the memory driver for this workspace (kernel.md §3.1), on the
        // same `plan.memory` gate as the store above — the binding is part of
        // the memory domain's init, not a separate gate. Warmed here rather
        // than lazily so a bad `[subsystems.memory]` is loud at boot instead of
        // at the first recall. Infallible by design: an inadmissible driver
        // falls back, publishes `MemoryDriverBindFailed`, and records why.
        match crate::memory::binding::for_config(cfg) {
            Ok(binding) => log::info!(
                "[boot] memory driver bound: id={} class={} capabilities=[{}] fallback={:?}",
                binding.driver_id(),
                binding.class(),
                binding
                    .capabilities()
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                binding.fallback().map(|f| f.reason.as_str()),
            ),
            Err(e) => log::warn!("[boot] memory driver bind failed: {e}"),
        }
    } else {
        log::debug!("[boot] memory driver bind SKIPPED — Memory domain disabled");
    }
    // Install the on-disk image-attachment sidecar dir so inbound
    // image markers persist under <workspace>/attachments/ instead
    // of an in-memory FIFO (survives restarts + delegation hops).
    // Also fires a best-effort stale-file sweep.
    if plan.agent_attachments {
        crate::agent::multimodal::init_attachments_dir(cfg.workspace_dir.join("attachments"));
        log::info!(
            "[boot] image attachments sidecar dir = {}",
            cfg.workspace_dir.join("attachments").display()
        );
    } else {
        log::debug!("[boot] image attachments sidecar dir SKIPPED — Agent domain disabled");
    }
    // (The WhatsApp data store moved to the Tauri shell; the core no longer
    // initializes it here. The shell lazily opens it from its own workspace
    // dir when the first ingest / query arrives.)
    // The people store is NOT seeded here any more. People is served by the
    // bound memory driver (`MemoryPeople`), so the engine owns that database —
    // and the module opens it. Seeding a host-side process-global as well meant
    // two readers over one SQLite file, with nothing left reading the host's:
    // `CoreContext::people()` is gone and no handler consults
    // `people::store::get()`.
    // Prune legacy bundled skills (dev-workflow / github-issue-crusher
    // / pr-review-shepherd) that older builds seeded into
    // <workspace>/skills/. OpenHuman no longer ships bundled defaults;
    // this removes the stale dirs on upgrade. Idempotent.
    if plan.skills_prune {
        crate::skills::registry::prune_legacy_default_workflows(&cfg.workspace_dir);
    } else {
        log::debug!("[boot] skills legacy-workflow prune SKIPPED — Skills domain disabled");
    }
    // Boot-time Sentry user binding — issue #3135. If the user is
    // already signed in (typical desktop restart), the auth-profile
    // store has their `user_id` *now*, before any background loop
    // (Composio sync tick, cron, etc.) fires its first event.
    // Reading from the store here means subsequent events carry
    // `user.id` even when no `app_state_snapshot` RPC has run yet.
    match crate::security::credentials::session_support::build_session_state(cfg) {
        Ok(state) => {
            if let Some(uid) = state.user_id.as_deref() {
                crate::security::credentials::sentry_scope::bind(uid);
            }
            // The host-supplied user payload survives restarts in the profile
            // store; seed the identity slot from it so prompt composition sees
            // the signed-in user before any host RPC runs.
            crate::security::credentials::identity::set_current_user(state.user);
        }
        Err(e) => {
            log::debug!("[boot] sentry scope user bind skipped — build_session_state failed: {e}")
        }
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
