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

use crate::core::types::HostKind;

#[path = "context_accessors.rs"]
mod accessors;

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
    /// The workspace this context is bound to. Shared (`Arc`) so a context
    /// derived over the same workspace follows the parent when the active
    /// user's workspace is rebound.
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
    latest_embedder_config: Option<Arc<std::sync::Mutex<crate::config::Config>>>,
    /// Per-tool-group disclosure for this context (see
    /// [`ToolGroups`](crate::tools::toolpacks::ToolGroups)).
    ///
    /// The third narrowing axis, independent of `domains` the same way
    /// `DomainSet` is independent of `ServiceSet`: `DomainSet` decides which
    /// families *exist*, `ToolGroups` decides how the ones that exist reach
    /// the model. Defaults to every group withheld, which is what the
    /// compiled-in pack table meant before the type existed.
    tool_groups: crate::tools::toolpacks::ToolGroups,
    /// Whether autonomy-tier checks reload config or retain this context's
    /// effective in-memory tier.
    autonomy_tier_source: super::AutonomyTierSource,
    /// A fixed maximum autonomy level inherited by derived contexts.
    autonomy_tier_ceiling: Option<crate::security::AutonomyLevel>,
    /// Which tiny dependencies the config switched off (`[components]`). Only
    /// narrows: `domains` and `tool_groups` are already clamped by it, and this
    /// is kept so dispatch can answer `COMPONENT_DISABLED:<name>` for a method
    /// that a disabled component removed.
    components: crate::core::runtime::ComponentSet,
    /// Whether skill discovery under this context scans the operator's
    /// user-scope roots (`~/.openhuman/skills`, `~/.agents/skills`).
    ///
    /// `true` for every booted context — the desktop, CLI and a plain library
    /// embedder all run as the operator. A per-agent context derived through
    /// [`CoreContext::derive_with`] can turn it off so an embedded agent sees
    /// only the skills its host installed for it, never the operator's.
    user_skill_roots: bool,
    /// Hook registrations bound to this context (see [`crate::core::slots`]).
    /// Empty unless the host supplied some through
    /// [`CoreBuilder::extensions`](crate::core::runtime::CoreBuilder::extensions);
    /// inherited by every context derived through [`CoreContext::derive_with`].
    extensions: crate::core::slots::Extensions,
    /// Explicit authority on this derived turn context.
    turn_origin: Option<crate::core::runtime::turn_origin::AgentTurnOrigin>,
    /// Durable relay conversation carried by the immutable turn context.
    channel_thread: Option<String>,
    /// The agent whose stores a host-installed session store
    /// ([`crate::storage::sessions`]) hands out to work under this
    /// context. `None` for booted contexts, which use the shared default.
    session_agent: Option<String>,
    /// The tenant (SaaS profile) this context serves. `None` for booted
    /// contexts and every single-user agent. See [`crate::core::runtime::tenant`].
    profile: Option<String>,
    /// Private admission generation shared only by contexts derived within
    /// this profile generation. Profile reopen creates a fresh context from
    /// the host root and therefore a fresh token.
    profile_admission: Arc<()>,
    /// What the agent this context was derived for owns.
    agent: agent_parts::AgentParts,
    /// A context that exists only to carry a turn origin where no core was
    /// booted ([`CoreContext::bare`]). Every reader other than the turn-origin
    /// ones treats it as if nothing were scoped: [`CoreContext::current`] and
    /// [`CoreContext::scoped`] skip it.
    bare: bool,
}

/// The workspace a context is bound to.
struct WorkspaceBinding {
    workspace_dir: Option<std::path::PathBuf>,
    config_path: Option<std::path::PathBuf>,
}

impl CoreContext {
    /// The host that constructed this context (Tauri shell / CLI / Docker).
    pub fn host_kind(&self) -> HostKind {
        self.host_kind
    }

    /// Which domain families are live for this context (#4796). The controller
    /// registry consults this (via [`CoreContext::current`]) to filter its
    /// schema/dispatch/tool surface. `full()` for desktop/CLI.
    /// The `[components]` switches this context booted with.
    pub fn components(&self) -> crate::core::runtime::ComponentSet {
        self.components
    }

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

    /// Derive a child with its own config and narrowed tool/runtime surfaces.
    /// The child shares host identity but does not boot stores or alter the
    /// process default; scoped readers observe its config and policy.
    pub fn derive_with(&self, overlay: ContextOverlay) -> Arc<CoreContext> {
        let parent_config = self.latest_embedder_config();
        self.derive_with_parent_config_inner(parent_config.as_ref(), overlay, false)
    }

    /// Derive a child using the exact parent configuration it was built from.
    ///
    /// This is used by embedders that load their config after core boot: the
    /// context may intentionally have no published embedder snapshot, while
    /// the child still needs the correct baseline for conflict-refresh
    /// overrides.
    pub fn derive_with_parent_config(
        &self,
        parent_config: &crate::config::Config,
        overlay: ContextOverlay,
    ) -> Arc<CoreContext> {
        self.derive_with_parent_config_inner(Some(parent_config), overlay, false)
    }

    /// Whether `other` descends from the same admitted profile context
    /// generation. This differs from context pointer identity: turn contexts
    /// may be derived while the profile remains admitted.
    pub(crate) fn shares_profile_admission(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.profile_admission, &other.profile_admission)
    }

    fn derive_with_parent_config_inner(
        &self,
        parent_config: Option<&crate::config::Config>,
        mut overlay: ContextOverlay,
        fresh_state: bool,
    ) -> Arc<CoreContext> {
        // Clamped to what this context can already dispatch. A derived
        // overlay may only narrow: callers outside this crate (embed's
        // `Runtime::agent`, for one) already refuse a spec that names a
        // family the runtime never registered, but that check lives at the
        // call site. Enforcing it here too means a future or third-party
        // caller of `derive_with` cannot widen a restricted parent's domains
        // just by omitting that check.
        let domains = self.domains.intersect(&overlay.domains);
        let components = if fresh_state {
            crate::core::runtime::ComponentSet::from_config(&overlay.config.components)
        } else {
            self.components
        };
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
        // A derived context over the parent's workspace and config root shares
        // its current binding snapshot. A later parent rebind replaces the
        // parent's handle, leaving already-derived work on the prior binding.
        // Another workspace or config root gets its own binding from the start.
        let shared_binding = {
            let parent_handle = self.workspace_binding.read();
            let parent_handle = parent_handle
                .as_ref()
                .ok()
                .map(|handle| Arc::clone(&**handle));
            let can_share = parent_handle.as_ref().is_some_and(|handle| {
                handle.read().ok().is_some_and(|parent| {
                    parent.workspace_dir.as_deref() == Some(overlay.config.workspace_dir.as_path())
                        && parent.config_path.as_deref()
                            == Some(overlay.config.config_path.as_path())
                })
            });
            match (can_share, parent_handle.as_ref()) {
                (true, Some(handle)) => Arc::clone(handle),
                _ => Arc::new(RwLock::new(WorkspaceBinding {
                    workspace_dir: Some(overlay.config.workspace_dir.clone()),
                    config_path: Some(overlay.config.config_path.clone()),
                })),
            }
        };
        let agent = self.agent.derive(&mut overlay, fresh_state);
        config_access::record_context_config_overlay(self.profile(), parent_config, &mut overlay);
        overlay.config.runtime_state.context_snapshot = None;
        let latest_embedder_config = Some(Arc::new(std::sync::Mutex::new(overlay.config.clone())));
        if let Some(snapshot) = &latest_embedder_config {
            crate::config::Config::bind_context_snapshot(snapshot);
        }
        let profile_admission = if overlay
            .profile
            .as_deref()
            .is_none_or(|profile| Some(profile) == self.profile())
        {
            Arc::clone(&self.profile_admission)
        } else {
            Arc::new(())
        };
        Arc::new(CoreContext {
            host_kind: self.host_kind,
            workspace_binding: RwLock::new(shared_binding),
            domains,
            tool_groups: overlay.tool_groups,
            components,
            autonomy_tier_source: overlay
                .autonomy_tier_source
                .unwrap_or(self.autonomy_tier_source),
            autonomy_tier_ceiling: overlay::stricter_autonomy_ceiling(
                self.autonomy_tier_ceiling,
                overlay.autonomy_tier_ceiling,
            ),
            embedder_config: Some(overlay.config),
            latest_embedder_config,
            user_skill_roots: overlay.user_skill_roots,
            extensions: self.extensions.clone(),
            turn_origin: self.turn_origin.clone(),
            channel_thread: self.channel_thread.clone(),
            session_agent: overlay.session_agent.or_else(|| self.session_agent.clone()),
            profile: overlay.profile.or_else(|| self.profile.clone()),
            profile_admission,
            agent,
            bare: self.bare,
        })
    }

    /// The agent a host session store scopes work under this context to, if
    /// this context was derived for one.
    pub fn session_agent(&self) -> Option<&str> {
        self.session_agent.as_deref()
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

    /// The workspace and config path hint from one binding snapshot. The hint
    /// is captured during normal boot even when no embedder config was supplied.
    pub(crate) fn workspace_config_binding(
        &self,
    ) -> Result<(std::path::PathBuf, Option<std::path::PathBuf>), String> {
        let handle = self
            .workspace_binding
            .read()
            .map_err(|e| format!("workspace unavailable: context lock poisoned: {e}"))?;
        let binding = handle
            .read()
            .map_err(|e| format!("workspace unavailable: binding lock poisoned: {e}"))?;
        let workspace = binding.workspace_dir.clone().ok_or_else(|| {
            "workspace unavailable: Config::load_or_init failed during core boot; \
             fix config.toml or OPENHUMAN_WORKSPACE and restart"
                .to_string()
        })?;
        Ok((workspace, binding.config_path.clone()))
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

    /// Rebind the process default context to the current active user's
    /// workspace. Desktop login, logout, and pending-session revalidation can
    /// switch the active workspace after boot without rebuilding the core.
    /// Scoped multi-tenant dispatch is unaffected because tenant contexts are
    /// passed to [`CoreContext::scope`] explicitly and are not the process
    /// default.
    pub fn rebind_default_workspace(workspace_dir: &std::path::Path) -> Result<(), String> {
        Self::rebind_default_workspace_with_config(workspace_dir, None)
    }

    pub(crate) fn rebind_default_workspace_with_config(
        workspace_dir: &std::path::Path,
        config_path: Option<&std::path::Path>,
    ) -> Result<(), String> {
        let Some(ctx) = DEFAULT_CONTEXT.get() else {
            log::debug!(
                "[core-context] default context not initialized; skipped workspace rebind to {}",
                workspace_dir.display()
            );
            return Ok(());
        };
        ctx.rebind_workspace(workspace_dir, config_path)
    }

    pub(crate) fn rebind_workspace(
        &self,
        workspace_dir: &std::path::Path,
        config_path: Option<&std::path::Path>,
    ) -> Result<(), String> {
        let mut binding_handle = self
            .workspace_binding
            .write()
            .map_err(|e| format!("workspace rebind failed: binding handle lock poisoned: {e}"))?;
        let already = {
            let binding = binding_handle
                .read()
                .map_err(|e| format!("workspace rebind failed: binding lock poisoned: {e}"))?;
            binding.workspace_dir.as_deref() == Some(workspace_dir)
                && binding.config_path.as_deref() == config_path
        };
        if already {
            log::debug!(
                "[core-context] workspace {} already bound",
                workspace_dir.display()
            );
            return Ok(());
        }
        log::info!(
            "[core-context] rebound default workspace to {}",
            workspace_dir.display()
        );
        *binding_handle = Arc::new(RwLock::new(WorkspaceBinding {
            workspace_dir: Some(workspace_dir.to_path_buf()),
            config_path: config_path.map(std::path::Path::to_path_buf),
        }));
        Ok(())
    }

    /// Run `fut` with `ctx` as the ambient [`CoreContext::current`]: dispatch wraps
    /// each handler in this, and multi-tenant hosts pass the tenant's context.
    ///
    /// A turn's origin survives a change of context: `ctx` without an origin of
    /// its own, scoped inside a turn that has one (an embedded agent's derived
    /// context replacing the caller's, say), keeps that turn's origin. Dropping
    /// it would turn a labelled turn into an unlabelled one the approval gate
    /// refuses.
    pub async fn scope<F: Future>(ctx: Arc<CoreContext>, fut: F) -> F::Output {
        CURRENT_CONTEXT
            .scope(Self::keeping_turn_origin(ctx), fut)
            .await
    }

    /// [`scope`](Self::scope) for a synchronous closure.
    pub fn sync_scope<R>(ctx: Arc<CoreContext>, f: impl FnOnce() -> R) -> R {
        CURRENT_CONTEXT.sync_scope(Self::keeping_turn_origin(ctx), f)
    }

    /// `ctx`, carrying the origin of the turn it is being scoped inside when it
    /// has none of its own.
    fn keeping_turn_origin(ctx: Arc<CoreContext>) -> Arc<CoreContext> {
        if ctx.turn_origin.is_some() {
            return ctx;
        }
        match Self::scoped_for_turn().and_then(|outer| outer.turn_origin.clone()) {
            Some(origin) => ctx.derive_channel_turn(None, Some(origin)),
            None => ctx,
        }
    }

    /// Capture the current context now and carry it across a subsequently
    /// spawned task. Calling this before `tokio::spawn` is essential: reading
    /// `current()` inside the child would already have fallen back to the
    /// process default.
    ///
    /// In SaaS only the caller's own task scope is carried (never the
    /// operator's default context), together with its memory identity — the
    /// same rule as [`DispatchCtx::spawn`](crate::core::runtime::DispatchCtx::spawn).
    pub fn propagate<F: Future>(fut: F) -> impl Future<Output = F::Output> {
        super::spawn::carry(fut)
    }
}

/// Initialize the workspace-bound stores and report the memory engine.
///
/// A `Config::load_or_init` failure here is operator-visible and serious
/// (corrupt toml, bad permissions, missing/unwritable `OPENHUMAN_WORKSPACE` —
/// common on headless/containerised deploys with no writable `$HOME`).
/// Instead of falling back to `Config::default()` against the *wrong*
/// workspace (Sentry OPENHUMAN-CORE-48), the workspace-bound init is skipped
/// entirely; the server still comes up and the operator sees the loud error.
/// Per-`DomainGroup` gating decision for each workspace-bound store that
#[path = "context_store_init.rs"]
mod store_init;
pub use store_init::{init_stores, StoreInitPlan};

#[path = "context_boot.rs"]
mod boot;

#[path = "context_turn_origin.rs"]
mod turn_origin_scope;

#[path = "context_for_agent.rs"]
mod for_agent_scope;

#[path = "context_agent.rs"]
mod agent_parts;

#[path = "context_inspection.rs"]
mod inspection;

#[path = "context_overlay.rs"]
mod overlay;
pub use overlay::{AutonomyTierSource, ContextOverlay};

#[path = "context_config.rs"]
mod config_access;

#[cfg(test)]
#[path = "context_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
