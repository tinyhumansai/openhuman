use std::path::Path;
use std::sync::Arc;

use super::super::background_completions::{
    entry_for, router_pool, router_scope, state, test_store_overrides, HostState,
};
use crate::core::runtime::tenant;
use tinyagents_tasks::CompletionStore;

/// A temporary workspace that forgets its router when dropped, so the
/// process-wide registry does not accumulate entries for deleted directories.
pub(crate) struct TestWorkspace(tempfile::TempDir);

impl TestWorkspace {
    pub(crate) fn new() -> Self {
        Self(tempfile::tempdir().expect("tempdir"))
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        self.0.path()
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        forget_workspace_for_test(self.0.path());
    }
}

/// Register `store` as `workspace_dir`'s completion store, so a test can inject
/// one that fails.
pub(crate) fn install_store_for_test(workspace_dir: &Path, store: Arc<dyn CompletionStore>) {
    let scope = router_scope(workspace_dir);
    test_store_overrides()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(scope, store);
    router_pool()
        .remove(&router_scope(workspace_dir))
        .expect("remove the scoped completion router");
    let _ = entry_for(workspace_dir);
}

/// Forget everything this process knows about `workspace_dir`, as a restart
/// would: the router (and its open log handle) and every thread/session mapping
/// that points at it. The on-disk log is untouched.
pub(crate) fn forget_workspace_for_test(workspace_dir: &Path) {
    let ambient_scope = router_scope(workspace_dir);
    test_store_overrides()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&ambient_scope);
    let mut st = state();
    let scopes: Vec<_> = st
        .stores
        .keys()
        .filter(|scope| scope.workspace_dir() == workspace_dir)
        .cloned()
        .collect();
    for scope in &scopes {
        st.stores.remove(scope);
    }
    let gone: Vec<String> = st
        .thread_workspaces
        .iter()
        .filter(|(_, ws)| ws.as_path() == workspace_dir)
        .map(|(thread, _)| thread.clone())
        .collect();
    for thread in &gone {
        st.thread_workspaces.remove(thread);
        st.stopped_threads.remove(thread);
        st.deleted_threads.remove(thread);
    }
    st.recovered_workspaces
        .retain(|scope| scope.workspace_dir() != workspace_dir);
    // Only the sessions of this workspace's threads. Clearing the whole cache
    // raced every test running beside it (the session-cache eviction test saw
    // its survivors vanish), and those tests do not take this lock.
    let HostState {
        session_threads,
        session_order,
        ..
    } = &mut *st;
    session_threads.retain(|session_key, thread| {
        tenant::split_key(session_key).map_or(true, |(owner, _)| {
            !gone.contains(&tenant::tenant_key(&owner, thread))
        })
    });
    session_order.retain(|session_key| session_threads.contains_key(session_key));
    drop(st);
    for scope in scopes {
        let _ = router_pool().remove(&scope);
    }
    let _ = router_pool().remove(&ambient_scope);
}

#[test]
fn forgetting_one_workspace_keeps_other_cached_routers() {
    let first = TestWorkspace::new();
    let second = TestWorkspace::new();
    let second_router = entry_for(second.path()).router.clone();

    forget_workspace_for_test(first.path());

    let still_cached = router_pool()
        .get(&router_scope(second.path()))
        .expect("lookup second workspace router")
        .expect("second workspace remains cached");
    assert!(Arc::ptr_eq(&still_cached, &second_router));
}
