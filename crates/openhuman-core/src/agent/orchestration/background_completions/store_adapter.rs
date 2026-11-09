//! Storage and router-pool adapters for background completion records.

use super::*;
use tinyagents_tasks::{
    open_driver_completion_store_or_memory, CompletionRouterPool, CompletionStoreFallback,
    CompletionStoreReplaceOutcome, InMemoryCompletionStore, DEFAULT_MAX_ATTEMPTS,
};

/// Presents OpenHuman's configured backend through TinyAgents' storage-kit
/// adapter while preserving native conditional writes.
struct CompletionBackend(Arc<dyn crate::storage::StorageBackend>);

impl tinyagents_storage_kit::StorageBackend for CompletionBackend {
    fn for_scope(
        &self,
        scope: &tinyagents_storage_kit::Scope,
    ) -> tinyagents_storage_kit::error::Result<tinyagents_storage_kit::ScopedStorage> {
        let host_scope = crate::storage::Scope::new(scope.as_str())?;
        let storage = self.0.for_scope(&host_scope)?;
        Ok(tinyagents_storage_kit::ScopedStorage::new(
            storage,
            scope.clone(),
        ))
    }

    fn for_scope_conditioned(
        &self,
        scope: &tinyagents_storage_kit::Scope,
        condition: &tinyagents_storage_kit::WriteCondition,
    ) -> tinyagents_storage_kit::error::Result<tinyagents_storage_kit::ScopedStorage> {
        let host_scope = crate::storage::Scope::new(scope.as_str())?;
        let storage = self.0.for_scope_conditioned(&host_scope, condition)?;
        Ok(tinyagents_storage_kit::ScopedStorage::new(
            storage,
            scope.clone(),
        ))
    }
}

pub(in crate::agent::orchestration) fn router_scope_for(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
) -> RouterScope {
    let profile = crate::core::runtime::tenant::isolated_unless_tenant(
        ctx.tenant(),
        "background_completions",
    )
    .profile_only();
    RouterScope {
        profile_key: crate::core::runtime::tenant::profile_key_in(&profile, ""),
        workspace_dir: workspace_dir.to_path_buf(),
    }
}

#[cfg(test)]
pub(in crate::agent::orchestration) fn router_scope(workspace_dir: &Path) -> RouterScope {
    router_scope_for(&DispatchCtx::ambient(), workspace_dir)
}

/// The durable completion queue of `workspace_dir` in the storage backend,
/// under the calling tenant's scope.
pub(in crate::agent::orchestration) fn open_driver_store(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
) -> anyhow::Result<Arc<dyn CompletionStore>> {
    with_storage_context(ctx, || {
        // Preserve the `background_completions` domain scope used by the
        // previous document-backed adapter. The owner API now needs the
        // backend and exact scope so it can provide its own fenced store.
        let opened = crate::storage::local::open("background_completions", workspace_dir)?;
        let fallback = if crate::storage::installed_is_shared() {
            CompletionStoreFallback::FailClosed
        } else {
            CompletionStoreFallback::LocalMemory
        };
        let scope = tinyagents_storage_kit::Scope::new(opened.scope.as_str())?;
        open_driver_completion_store_or_memory(
            Arc::new(CompletionBackend(opened.backend)),
            scope,
            fallback,
        )
        .map_err(|error| anyhow::anyhow!("open the completion store: {error}"))
    })
}

/// Scope storage access to the explicit context when one exists. A genuinely
/// unbound non-SaaS caller with no ambient profile may still use the ordinary
/// local-workspace backend; SaaS and foreign ambient scopes fail closed.
fn with_storage_context<R>(
    ctx: &DispatchCtx,
    open: impl FnOnce() -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    if let Some(core_context) = ctx.resolved() {
        return crate::core::runtime::CoreContext::sync_scope(core_context, open);
    }
    if !crate::core::runtime::is_saas() && crate::core::runtime::CoreContext::scoped().is_none() {
        return open();
    }
    Err(anyhow::anyhow!(
        "completion store requires a resolved dispatch context in this scope"
    ))
}

pub(in crate::agent::orchestration) fn router_pool() -> &'static CompletionRouterPool<RouterScope> {
    static POOL: OnceLock<CompletionRouterPool<RouterScope>> = OnceLock::new();
    POOL.get_or_init(|| {
        CompletionRouterPool::new(|scope: &RouterScope| {
            #[cfg(test)]
            if let Some(store) = test_store_overrides()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(scope)
            {
                return Arc::new(new_router(store));
            }
            // `get_or_open` invokes its opener synchronously, so carry the
            // caller's explicit profile context into storage::local::open.
            let store = match open_driver_store(&DispatchCtx::ambient(), &scope.workspace_dir) {
                Ok(store) => store,
                Err(error) => {
                    log::error!(
                        "[background_completions] could not open the completion store for {}; using an \
                         in-memory queue (completions will not survive a restart): {error:#}",
                        scope.workspace_dir.display()
                    );
                    Arc::new(InMemoryCompletionStore::new())
                }
            };
            Arc::new(new_router(store))
        })
    })
}

#[cfg(test)]
pub(in crate::agent::orchestration) fn test_store_overrides(
) -> &'static Mutex<HashMap<RouterScope, Arc<dyn CompletionStore>>> {
    static STORES: OnceLock<Mutex<HashMap<RouterScope, Arc<dyn CompletionStore>>>> =
        OnceLock::new();
    STORES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(in crate::agent::orchestration) fn entry_for_ctx(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
) -> Arc<Entry> {
    let scope = router_scope_for(ctx, workspace_dir);
    let unbound_saas_without_foreign_scope = crate::core::runtime::is_saas()
        && ctx.resolved().is_none()
        && crate::core::runtime::CoreContext::scoped().is_none();
    let lookup = || {
        router_pool()
            .get_or_open(&scope)
            .map_err(|error| anyhow::anyhow!("{error}"))
    };
    let opened = if unbound_saas_without_foreign_scope {
        // The pool opener will fail closed in storage::local and cache its
        // in-memory fallback, rather than borrowing any other profile.
        lookup()
    } else {
        with_storage_context(ctx, lookup)
    };
    let router = match opened {
        Ok(router) => router,
        Err(error) => {
            log::error!("[background_completions] router pool lookup failed: {error}");
            Arc::new(new_router(Arc::new(InMemoryCompletionStore::new())))
        }
    };
    let store = router.store();
    state().stores.insert(scope, store.clone());
    Arc::new(Entry { router, store })
}

#[cfg(test)]
pub(in crate::agent::orchestration) fn entry_for(workspace_dir: &Path) -> Arc<Entry> {
    entry_for_ctx(&DispatchCtx::ambient(), workspace_dir)
}

/// Replace a workspace's failing durable store with an in-memory one that starts
/// from the same records, so delivery keeps working in this process. The log on
/// disk is left as it is and replays on the next boot.
///
/// The owner migrates the store under the same lock as claims and transitions,
/// so concurrent callers keep the same router, leases and parent markers.
/// `failed` is the router the caller just saw fail.
pub(in crate::agent::orchestration) fn degrade_to_memory(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    failed: &Arc<CompletionRouter>,
) -> Arc<Entry> {
    let scope = router_scope_for(ctx, workspace_dir);
    let previous = failed.store();
    let replacement: Arc<dyn CompletionStore> = Arc::new(InMemoryCompletionStore::new());
    match failed.replace_store_if_current(&previous, replacement) {
        Ok(CompletionStoreReplaceOutcome::Replaced(store)) => {
            {
                let mut st = state();
                st.degraded_scopes.insert(scope.clone());
                st.stores.insert(scope, store.clone());
            }
            log::error!(
                "[background_completions] degraded to an in-memory queue workspace_dir={}",
                workspace_dir.display()
            );
            Arc::new(Entry {
                router: Arc::clone(failed),
                store,
            })
        }
        Ok(CompletionStoreReplaceOutcome::Current(store)) => {
            state().stores.insert(scope, store.clone());
            Arc::new(Entry {
                router: Arc::clone(failed),
                store,
            })
        }
        Err(error) => {
            // A failed enumeration/copy leaves the old router and lease state
            // canonical. Keep it addressable and report the actual migration
            // error instead of silently replacing it with an empty queue.
            log::error!(
                "[background_completions] could not migrate completion records to memory workspace_dir={} error={error:#}",
                workspace_dir.display()
            );
            state().stores.insert(scope, previous.clone());
            Arc::new(Entry {
                router: Arc::clone(failed),
                store: previous,
            })
        }
    }
}

pub(in crate::agent::orchestration) fn new_router(
    store: Arc<dyn CompletionStore>,
) -> CompletionRouter {
    CompletionRouter::new(store)
        .with_formatter(Arc::new(BackgroundCompletionFormatter))
        .with_max_attempts(DEFAULT_MAX_ATTEMPTS)
}

/// The router for `workspace_dir`, opening it on first use.
pub(crate) fn router_for_workspace_ctx(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
) -> Arc<CompletionRouter> {
    entry_for_ctx(ctx, workspace_dir).router.clone()
}

#[cfg(test)]
#[cfg(test)]
pub(in crate::agent::orchestration) fn router_for_workspace(
    workspace_dir: &Path,
) -> Arc<CompletionRouter> {
    router_for_workspace_ctx(&DispatchCtx::ambient(), workspace_dir)
}
