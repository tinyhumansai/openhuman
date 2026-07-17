//! Core initialization context.
//!
//! [`CoreContext`] owns the core's initialization *order* (Phase 2, Stage A):
//! register controllers, load the master key, seed the RPC bearer, initialize
//! the workspace-bound stores, and run pure `bootstrap_core_runtime` registration. Today it is a
//! facade — the store init still targets the process globals — but centralizing
//! the sequence here is the seam the later stages build on (handler-threaded
//! context, per-context stores). See `docs/plans/pluggable-core/phase-2-corecontext.md`.
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
    workspace_dir: RwLock<Option<std::path::PathBuf>>,
    /// Which domain families are live for this context (#4796). The registry
    /// filters its controller/schema/dispatch surface by this set via
    /// [`CoreContext::current`] → [`CoreContext::domains`]. `full()` for the
    /// desktop shell / standalone CLI (byte-identical to pre-#4796).
    domains: crate::core::runtime::DomainSet,
}

impl CoreContext {
    /// Run the core initialization sequence and return the context plus whether
    /// an operator-supplied RPC bearer exists (for the public-bind safety check
    /// in `CoreRuntime::serve`) plus the loaded config, when boot reached
    /// workspace-bound init. Order is load-bearing and mirrors the original
    /// `run_server_inner` sequence:
    ///
    /// 1. register controllers, 2. master key, 3. AgentBox GMI provider,
    /// 4. seed RPC bearer, 5. workspace stores ([`init_stores`]),
    /// 6. pure runtime registration.
    pub async fn init(
        host_kind: HostKind,
        token: &TokenSource,
        domains: crate::core::runtime::DomainSet,
    ) -> anyhow::Result<(
        Arc<CoreContext>,
        bool,
        Option<crate::openhuman::config::Config>,
    )> {
        log::debug!("[core-context] init: host_kind={host_kind:?} domains={domains:?}");
        // 1. Ensure all controllers are registered before anything dispatches.
        let _ = crate::core::all::all_registered_controllers();

        // 2. Load the master encryption key before any config/credential op that
        //    needs to decrypt secrets. No-op if already called (e.g. from
        //    run_core_from_args for the CLI).
        crate::openhuman::keyring::init_master_key();

        // 3. AgentBox GMI MaaS provider bridge — no-op when env vars absent. Must
        //    run before the router mounts the AgentBox routes so the inference
        //    catalog knows about "gmi-maas" by the time `/run` accepts traffic.
        crate::openhuman::agentbox::register_gmi_provider_if_present();

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
                let token_dir = crate::openhuman::config::default_root_openhuman_dir()
                    .unwrap_or_else(|_| {
                        dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join(".openhuman")
                    });
                crate::core::auth::init_rpc_token(&token_dir)?;
                std::env::var(crate::core::auth::CORE_TOKEN_ENV_VAR)
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .is_some()
            }
        };

        // 5. Resolve config once, then initialize workspace-bound stores
        //    (memory, attachments, whatsapp, people) with that exact workspace.
        let config = match crate::openhuman::config::Config::load_or_init().await {
            Ok(cfg) => {
                init_stores(&cfg, domains).await;
                Some(cfg)
            }
            Err(e) => {
                log::error!(
                    "[boot] memory::global + whatsapp_data init SKIPPED — \
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
        //    background jobs start later, from CoreRuntime::serve(), after bind
        //    succeeds.
        let runtime_config = config.clone();
        crate::core::jsonrpc::bootstrap_core_runtime(host_kind, config, domains).await;

        let ctx = Arc::new(CoreContext {
            host_kind,
            workspace_dir: RwLock::new(workspace_dir),
            domains,
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
    pub fn domains(&self) -> crate::core::runtime::DomainSet {
        self.domains
    }

    /// The resolved per-user workspace directory this context is bound to.
    pub fn workspace_dir(&self) -> Result<std::path::PathBuf, String> {
        self.workspace_dir
            .read()
            .map_err(|e| format!("workspace unavailable: context lock poisoned: {e}"))?
            .clone()
            .ok_or_else(|| {
                "workspace unavailable: Config::load_or_init failed during core boot; \
                 fix config.toml or OPENHUMAN_WORKSPACE and restart"
                    .to_string()
            })
    }

    /// The people store for this context's workspace — the first per-domain
    /// store handle carved off the process globals (Phase 2 Stage C /
    /// store-trait seam). Two contexts over different workspaces get isolated
    /// stores; the same context always gets the same cached store. Handlers
    /// migrate off `people::store::get()` by reading through
    /// `CoreContext::current()?.people()` instead.
    pub fn people(&self) -> Result<Arc<crate::openhuman::people::store::PeopleStore>, String> {
        let workspace_dir = self.workspace_dir()?;
        crate::openhuman::people::store::for_workspace(&workspace_dir)
    }

    /// The context for the current dispatch: the one scoped by
    /// [`CoreContext::scope`] if inside a scope, else the process
    /// [`DEFAULT_CONTEXT`]. Returns `None` only before any context is built
    /// (e.g. a unit test that dispatches without initializing the core).
    ///
    /// Handlers migrating off process globals read their state through this.
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
    /// workspace. Desktop login and pending-session revalidation can switch the
    /// active workspace after boot without rebuilding the core. Scoped
    /// multi-tenant dispatch is unaffected because tenant contexts are passed to
    /// [`CoreContext::scope`] explicitly and are not the process default.
    pub fn rebind_default_workspace_dir(workspace_dir: &std::path::Path) -> Result<(), String> {
        let Some(ctx) = DEFAULT_CONTEXT.get() else {
            log::debug!(
                "[core-context] default context not initialized; skipped workspace rebind to {}",
                workspace_dir.display()
            );
            return Ok(());
        };
        ctx.rebind_workspace_dir(workspace_dir)
    }

    fn rebind_workspace_dir(&self, workspace_dir: &std::path::Path) -> Result<(), String> {
        let mut guard = self
            .workspace_dir
            .write()
            .map_err(|e| format!("workspace rebind failed: context lock poisoned: {e}"))?;
        if guard.as_deref() == Some(workspace_dir) {
            log::debug!(
                "[core-context] workspace already bound to {}",
                workspace_dir.display()
            );
            return Ok(());
        }
        log::info!(
            "[core-context] rebound default workspace to {}",
            workspace_dir.display()
        );
        *guard = Some(workspace_dir.to_path_buf());
        Ok(())
    }

    /// Run `fut` with `ctx` as the ambient [`CoreContext::current`]. The dispatch
    /// layer wraps each handler invocation in this; multi-tenant hosts pass the
    /// tenant's context here so the handler's `current()` reads isolated state.
    pub async fn scope<F: Future>(ctx: Arc<CoreContext>, fut: F) -> F::Output {
        CURRENT_CONTEXT.scope(ctx, fut).await
    }

    /// Test-only constructor: build a context with an explicit
    /// [`DomainSet`](crate::core::runtime::DomainSet) and optional workspace, so
    /// cross-module tests (e.g. `core::all`'s registry filter) can exercise the
    /// ambient DomainSet gate without going through the full [`CoreContext::init`]
    /// boot sequence.
    #[cfg(test)]
    pub(crate) fn for_test(
        domains: crate::core::runtime::DomainSet,
        workspace_dir: Option<std::path::PathBuf>,
    ) -> Arc<CoreContext> {
        Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_dir: RwLock::new(workspace_dir),
            domains,
        })
    }
}

/// Initialize the global `MemoryClient` and the other workspace-bound stores so
/// composio providers (gmail/slack/notion) can persist their `sync_state`, and
/// so any subsystem that calls `memory::global::client_if_ready()` gets a live
/// handle.
///
/// A `Config::load_or_init` failure here is operator-visible and serious
/// (corrupt toml, bad permissions, missing/unwritable `OPENHUMAN_WORKSPACE` —
/// common on headless/containerised deploys with no writable `$HOME`).
/// Previously the fallback to `Config::default()` initialised the memory +
/// whatsapp_data stores against the *wrong* workspace dir, silently causing
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
    /// `memory::global` — gated on [`DomainGroup::Memory`].
    pub memory: bool,
    /// `agent::multimodal` attachments sidecar dir — gated on [`DomainGroup::Agent`].
    pub agent_attachments: bool,
    /// `whatsapp_data::global` — gated on [`DomainGroup::Channels`].
    pub whatsapp_data: bool,
    /// `people::store` — gated on [`DomainGroup::Platform`].
    pub people: bool,
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
            whatsapp_data: domains.allows(DomainGroup::Channels),
            people: domains.allows(DomainGroup::Platform),
            skills_prune: domains.allows(DomainGroup::Skills),
        }
    }
}

pub async fn init_stores(
    cfg: &crate::openhuman::config::Config,
    domains: crate::core::runtime::DomainSet,
) {
    let plan = StoreInitPlan::for_domains(domains);

    let keyring_dir = crate::openhuman::keyring::store::workspace_dir_for_file_backend();
    // Keyring path log + credentials Sentry bind (below) are unguarded — they
    // are core infra every DomainSet needs. Each workspace-bound store init is
    // gated on its owning DomainGroup so an excluded domain's store stays
    // uninitialized under `harness()`/`none()` (#4796 DoD item 3).
    log::info!(
        "[boot] paths: config={} workspace={} keyring_dir={} keyring_backend={} domains={:?}",
        cfg.config_path.display(),
        cfg.workspace_dir.display(),
        keyring_dir.display(),
        crate::openhuman::keyring::backend_name(),
        domains,
    );
    if plan.memory {
        match crate::openhuman::memory::global::init(cfg.workspace_dir.clone()) {
            Ok(_) => log::info!(
                "[boot] memory::global initialized (workspace={})",
                cfg.workspace_dir.display()
            ),
            Err(e) => log::warn!("[boot] memory::global init failed: {e}"),
        }
    } else {
        log::debug!("[boot] memory::global init SKIPPED — Memory domain disabled");
    }
    // Install the on-disk image-attachment sidecar dir so inbound
    // image markers persist under <workspace>/attachments/ instead
    // of an in-memory FIFO (survives restarts + delegation hops).
    // Also fires a best-effort stale-file sweep.
    if plan.agent_attachments {
        crate::openhuman::agent::multimodal::init_attachments_dir(
            cfg.workspace_dir.join("attachments"),
        );
        log::info!(
            "[boot] image attachments sidecar dir = {}",
            cfg.workspace_dir.join("attachments").display()
        );
    } else {
        log::debug!("[boot] image attachments sidecar dir SKIPPED — Agent domain disabled");
    }
    // Initialize the WhatsApp data store so scanner ingest calls
    // can write data without requiring a lazy-init fallback.
    // Compile-time `channels` gate: the body names `whatsapp_data::global`,
    // which is compiled out with the feature off, so the block is `#[cfg]`-gated.
    // The `plan.whatsapp_data` field + its runtime tests stay.
    #[cfg(feature = "channels")]
    if plan.whatsapp_data {
        match crate::openhuman::whatsapp_data::global::init(cfg.workspace_dir.clone()) {
            Ok(_) => log::info!(
                "[boot] whatsapp_data::global initialized (workspace={})",
                cfg.workspace_dir.display()
            ),
            Err(e) => log::warn!("[boot] whatsapp_data::global init failed: {e}"),
        }
    } else {
        log::debug!("[boot] whatsapp_data::global init SKIPPED — Channels domain disabled");
    }
    #[cfg(not(feature = "channels"))]
    log::debug!(
        "[boot] whatsapp_data::global init SKIPPED — channels feature disabled at compile time"
    );
    // Seed the people store so people controllers + `people_*`
    // tools can read/write. Without this the process-global stays
    // empty and every call fails with "people store not
    // initialised" (Sentry TAURI-RUST-8NM). Sits inside this
    // Ok(cfg) arm so it inherits the wrong-workspace guard above
    // (never seed against a Config::default fallback).
    if plan.people {
        match crate::openhuman::people::store::init_from_workspace(&cfg.workspace_dir) {
            Ok(_) => log::info!(
                "[boot] people::store initialized (workspace={})",
                cfg.workspace_dir.display()
            ),
            Err(e) => log::warn!("[boot] people::store init failed: {e}"),
        }
    } else {
        log::debug!("[boot] people::store init SKIPPED — Platform domain disabled");
    }
    // Prune legacy bundled skills (dev-workflow / github-issue-crusher
    // / pr-review-shepherd) that older builds seeded into
    // <workspace>/skills/. OpenHuman no longer ships bundled defaults;
    // this removes the stale dirs on upgrade. Idempotent.
    if plan.skills_prune {
        crate::openhuman::skills::registry::prune_legacy_default_workflows(&cfg.workspace_dir);
    } else {
        log::debug!("[boot] skills legacy-workflow prune SKIPPED — Skills domain disabled");
    }
    // Boot-time Sentry user binding — issue #3135. If the user is
    // already signed in (typical desktop restart), the auth-profile
    // store has their `user_id` *now*, before any background loop
    // (Composio sync tick, heartbeat, etc.) fires its first event.
    // Reading from the store here means subsequent events carry
    // `user.id` even when no `app_state_snapshot` RPC has run yet.
    match crate::openhuman::credentials::session_support::build_session_state(cfg) {
        Ok(state) => {
            if let Some(uid) = state.user_id.as_deref() {
                crate::openhuman::credentials::sentry_scope::bind(uid);
            }
        }
        Err(e) => {
            log::debug!("[boot] sentry scope user bind skipped — build_session_state failed: {e}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx(dir: &str) -> Arc<CoreContext> {
        Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_dir: RwLock::new(Some(PathBuf::from(dir))),
            domains: crate::core::runtime::DomainSet::full(),
        })
    }

    // The ambient-scope primitive is the mechanism Phase 3 multi-tenant
    // isolation is built on: a dispatch scoped to context A must see A's state,
    // not the process default or another tenant's. These assert the primitive
    // directly (independent of the process DEFAULT_CONTEXT global, since
    // `current()` inside a scope resolves the scoped value).

    // ---- store-init gating (#4796 DoD item 3) --------------------------------
    // `init_stores` side-effects on process globals with no init-state probe, so
    // the gating is proven via the pure `StoreInitPlan` the registrar consumes.

    #[test]
    fn store_init_plan_full_initializes_every_store() {
        let plan = StoreInitPlan::for_domains(crate::core::runtime::DomainSet::full());
        assert_eq!(
            plan,
            StoreInitPlan {
                memory: true,
                agent_attachments: true,
                whatsapp_data: true,
                people: true,
                skills_prune: true,
            },
            "full() must initialize every workspace-bound store"
        );
    }

    #[test]
    fn store_init_plan_none_initializes_nothing() {
        let plan = StoreInitPlan::for_domains(crate::core::runtime::DomainSet::none());
        assert_eq!(
            plan,
            StoreInitPlan {
                memory: false,
                agent_attachments: false,
                whatsapp_data: false,
                people: false,
                skills_prune: false,
            },
            "none() must leave every workspace-bound store uninitialized"
        );
    }

    #[test]
    fn store_init_plan_harness_gates_by_owning_group() {
        let plan = StoreInitPlan::for_domains(crate::core::runtime::DomainSet::harness());
        // harness() = agent + memory + threads + config + security.
        assert!(plan.memory, "harness keeps memory::global (Memory)");
        assert!(
            plan.agent_attachments,
            "harness keeps agent attachments sidecar (Agent)"
        );
        // Channels / Platform / Skills are NOT in harness → their stores stay off.
        assert!(
            !plan.whatsapp_data,
            "harness must skip whatsapp_data::global (Channels)"
        );
        assert!(!plan.people, "harness must skip people::store (Platform)");
        assert!(
            !plan.skills_prune,
            "harness must skip skills legacy-prune (Skills)"
        );
    }

    #[tokio::test]
    async fn scope_sets_current_context() {
        let a = ctx("/tmp/ctx-a");
        let seen = CoreContext::scope(a, async {
            CoreContext::current().map(|c| c.workspace_dir().unwrap())
        })
        .await;
        assert_eq!(seen, Some(PathBuf::from("/tmp/ctx-a")));
    }

    #[tokio::test]
    async fn scoped_context_exposes_its_domain_set() {
        // The ambient `current().domains()` must reflect the scoped context's
        // DomainSet — this is the seam the registry filter reads (#4796).
        let harness = crate::core::runtime::DomainSet::harness();
        let ctx = CoreContext::for_test(harness, Some(PathBuf::from("/tmp/ctx-domains")));
        let seen =
            CoreContext::scope(ctx, async { CoreContext::current().map(|c| c.domains()) }).await;
        assert_eq!(seen, Some(harness));
        assert!(seen.unwrap().allows(crate::core::all::DomainGroup::Memory));
        assert!(!seen.unwrap().allows(crate::core::all::DomainGroup::Web3));
    }

    #[tokio::test]
    async fn nested_scope_overrides_then_restores() {
        let a = ctx("/tmp/ctx-a");
        let b = ctx("/tmp/ctx-b");
        let (inner, outer) = CoreContext::scope(a, async {
            let inner = CoreContext::scope(b, async {
                CoreContext::current().unwrap().workspace_dir().unwrap()
            })
            .await;
            let outer = CoreContext::current().unwrap().workspace_dir().unwrap();
            (inner, outer)
        })
        .await;
        // Inner dispatch sees tenant B; the outer scope is restored to A after.
        assert_eq!(inner, PathBuf::from("/tmp/ctx-b"));
        assert_eq!(outer, PathBuf::from("/tmp/ctx-a"));
    }

    // The Phase 3 exit criterion, at the store level: two contexts over distinct
    // workspaces resolve isolated per-domain stores, and one context always
    // resolves the same cached store. This is the vertical proof that the
    // ambient-context mechanism + a per-context store handle give real
    // cross-context isolation (here for the first migrated domain, `people`).
    #[test]
    fn people_store_is_isolated_per_context_workspace() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_dir: RwLock::new(Some(dir_a.path().to_path_buf())),
            domains: crate::core::runtime::DomainSet::full(),
        });
        let b = Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_dir: RwLock::new(Some(dir_b.path().to_path_buf())),
            domains: crate::core::runtime::DomainSet::full(),
        });

        let store_a = a.people().expect("open people store for workspace A");
        let store_b = b.people().expect("open people store for workspace B");
        // Different workspaces → isolated stores.
        assert!(!Arc::ptr_eq(&store_a, &store_b));

        // Same context/workspace → same cached store (no per-call reopen).
        let store_a_again = a.people().expect("reopen people store for workspace A");
        assert!(Arc::ptr_eq(&store_a, &store_a_again));
    }

    #[test]
    fn rebind_workspace_updates_context_store_resolution() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let ctx = CoreContext {
            host_kind: HostKind::Cli,
            workspace_dir: RwLock::new(Some(dir_a.path().to_path_buf())),
            domains: crate::core::runtime::DomainSet::full(),
        };

        let store_a = ctx.people().expect("open people store for workspace A");
        ctx.rebind_workspace_dir(dir_b.path())
            .expect("rebind context workspace");

        assert_eq!(ctx.workspace_dir().unwrap(), dir_b.path());
        let store_b = ctx.people().expect("open people store for workspace B");
        assert!(!Arc::ptr_eq(&store_a, &store_b));
    }

    #[tokio::test]
    async fn people_rpc_uses_scoped_context_store() {
        use crate::openhuman::people::types::Handle;

        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_dir: RwLock::new(Some(dir_a.path().to_path_buf())),
            domains: crate::core::runtime::DomainSet::full(),
        });
        let b = Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_dir: RwLock::new(Some(dir_b.path().to_path_buf())),
            domains: crate::core::runtime::DomainSet::full(),
        });

        let params = serde_json::json!({
            "kind": "email",
            "value": "tenant-a@example.com",
            "create_if_missing": true
        })
        .as_object()
        .unwrap()
        .clone();

        let result = CoreContext::scope(
            a.clone(),
            crate::core::all::try_invoke_registered_rpc("openhuman.people_resolve", params),
        )
        .await
        .expect("people_resolve registered")
        .expect("people_resolve succeeds");

        assert_eq!(result["created"], true);
        let handle = Handle::Email("tenant-a@example.com".to_string());
        assert!(
            a.people()
                .expect("workspace A store")
                .lookup(&handle)
                .await
                .unwrap()
                .is_some(),
            "scoped RPC must write workspace A"
        );
        assert!(
            b.people()
                .expect("workspace B store")
                .lookup(&handle)
                .await
                .unwrap()
                .is_none(),
            "scoped RPC must not write workspace B"
        );
    }

    #[test]
    fn degraded_context_rejects_workspace_bound_stores() {
        let ctx = CoreContext {
            host_kind: HostKind::Cli,
            workspace_dir: RwLock::new(None),
            domains: crate::core::runtime::DomainSet::full(),
        };

        let err = match ctx.people() {
            Ok(_) => panic!("degraded context unexpectedly opened a people store"),
            Err(err) => err,
        };
        assert!(
            err.contains("workspace unavailable"),
            "unexpected error: {err}"
        );
    }
}
