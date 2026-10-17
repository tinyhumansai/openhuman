//! `load_config_in` reads the context it is handed, not the one the task has.

use super::*;
use crate::core::runtime::{context::CoreContext, DispatchCtx, DomainSet};
use std::sync::Arc;

fn tenant(model: &str) -> Arc<CoreContext> {
    let mut config = Config::default();
    config.default_model = Some(model.to_owned());
    CoreContext::for_test_with_config(DomainSet::full(), config)
}

#[tokio::test]
async fn the_explicit_context_wins_over_the_tasks_scope() {
    let scoped = tenant("scoped-model");
    let explicit = tenant("explicit-model");
    let loaded = CoreContext::scope(scoped, async {
        DispatchCtx::new(explicit).load_config().await
    })
    .await
    .expect("explicit context config loads");
    assert_eq!(loaded.default_model.as_deref(), Some("explicit-model"));
}

/// Two tenants loading at once, interleaving at an await point, each read only
/// their own config.
#[tokio::test(flavor = "current_thread")]
async fn concurrent_loads_never_see_each_others_config() {
    let a = DispatchCtx::new(tenant("tenant-a"));
    let b = DispatchCtx::new(tenant("tenant-b"));
    let load = |ctx: DispatchCtx| async move {
        tokio::task::yield_now().await;
        let config = ctx.load_config().await.expect("loads");
        tokio::task::yield_now().await;
        config.default_model
    };
    let (ra, rb) = tokio::join!(load(a), load(b));
    assert_eq!(ra.as_deref(), Some("tenant-a"));
    assert_eq!(rb.as_deref(), Some("tenant-b"));
}

/// A spawned task is handed the context by value and reads it after the
/// spawner's scope is gone.
#[tokio::test]
async fn a_spawned_task_loads_the_context_it_was_handed() {
    let ctx = DispatchCtx::new(tenant("spawned-model"));
    let loaded = tokio::spawn(async move { ctx.load_config().await })
        .await
        .expect("task joins")
        .expect("loads");
    assert_eq!(loaded.default_model.as_deref(), Some("spawned-model"));
}

#[tokio::test]
async fn the_ambient_loader_reads_the_scoped_context() {
    let loaded = CoreContext::scope(tenant("ambient-model"), load_config_with_timeout())
        .await
        .expect("loads");
    assert_eq!(loaded.default_model.as_deref(), Some("ambient-model"));
}

/// An op that takes its context (not just the handler) reads that context's
/// config: two tenants snapshotting at once, interleaved at await points, each
/// get their own, and the task's scope does not leak in.
#[tokio::test(flavor = "current_thread")]
async fn a_config_op_snapshots_the_context_it_is_handed() {
    let a = DispatchCtx::new(tenant("snapshot-tenant-a"));
    let b = DispatchCtx::new(tenant("snapshot-tenant-b"));
    let intruder = tenant("snapshot-scoped-intruder");
    let snapshot = |ctx: DispatchCtx| async move {
        tokio::task::yield_now().await;
        let out = load_and_get_config_snapshot(&ctx).await.expect("snapshot");
        out.value.to_string()
    };
    let (sa, sb) =
        CoreContext::scope(intruder, async { tokio::join!(snapshot(a), snapshot(b)) }).await;
    assert!(
        sa.contains("snapshot-tenant-a") && !sa.contains("snapshot-tenant-b"),
        "{sa}"
    );
    assert!(
        sb.contains("snapshot-tenant-b") && !sb.contains("snapshot-tenant-a"),
        "{sb}"
    );
    assert!(!sa.contains("scoped-intruder") && !sb.contains("scoped-intruder"));
}
