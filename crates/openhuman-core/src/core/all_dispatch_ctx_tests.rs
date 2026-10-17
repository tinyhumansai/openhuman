//! The dispatch context reaches the handler as an argument, per call.

use serde_json::{json, Map, Value};

use super::*;
use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
use std::sync::Arc;

/// Answers with the workspace of the context it was handed, after yielding so
/// another dispatch can interleave on the same thread.
fn whoami(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        tokio::task::yield_now().await;
        let bound = ctx.core().is_some();
        let workspace = ctx
            .core()
            .and_then(|core| core.workspace_dir().ok())
            .map(|dir| dir.display().to_string());
        // A spawned task is given the context by value; it cannot reach for one.
        let spawned = {
            let carried = ctx.clone();
            tokio::spawn(async move {
                carried
                    .core()
                    .and_then(|core| core.workspace_dir().ok())
                    .map(|dir| dir.display().to_string())
            })
            .await
            .map_err(|e| e.to_string())?
        };
        Ok(json!({ "bound": bound, "workspace": workspace, "spawned": spawned }))
    })
}

fn register_probe() {
    register_controller_extension(ControllerExtension {
        group: DomainGroup::Hosted,
        controllers: vec![RegisteredController {
            schema: schema("ext_ctx_probe", "whoami", vec![]),
            handler: whoami,
        }],
        namespaces: &[("ext_ctx_probe", "Dispatch context probe.")],
    })
    .expect("register ctx probe");
}

#[tokio::test]
async fn handler_receives_the_context_the_caller_dispatched_with() {
    register_probe();
    let ctx = CoreContext::for_test(DomainSet::full(), Some("/tmp/ctx-probe-a".into()));
    let value = try_invoke_registered_rpc(
        DispatchCtx::new(ctx),
        "openhuman.ext_ctx_probe_whoami",
        Map::new(),
    )
    .await
    .expect("dispatchable")
    .expect("handler succeeds");
    assert_eq!(value["bound"], json!(true));
    assert_eq!(value["workspace"], json!("/tmp/ctx-probe-a"));
    assert_eq!(
        value["spawned"],
        json!("/tmp/ctx-probe-a"),
        "a spawned task is handed the context explicitly"
    );
}

/// Two tenants dispatched at once, interleaving at an await point, each see only
/// their own context: nothing in the dispatch carries one call's value to the
/// other.
#[tokio::test(flavor = "current_thread")]
async fn concurrent_dispatches_never_see_each_others_context() {
    register_probe();
    let a = CoreContext::for_test(DomainSet::full(), Some("/tmp/ctx-probe-tenant-a".into()));
    let b = CoreContext::for_test(DomainSet::full(), Some("/tmp/ctx-probe-tenant-b".into()));
    let call = |ctx: Arc<CoreContext>| {
        try_invoke_registered_rpc(
            DispatchCtx::new(ctx),
            "openhuman.ext_ctx_probe_whoami",
            Map::new(),
        )
    };
    let (ra, rb) = tokio::join!(call(a), call(b));
    let (ra, rb) = (ra.unwrap().unwrap(), rb.unwrap().unwrap());
    assert_eq!(ra["workspace"], json!("/tmp/ctx-probe-tenant-a"));
    assert_eq!(rb["workspace"], json!("/tmp/ctx-probe-tenant-b"));
    assert_eq!(ra["spawned"], json!("/tmp/ctx-probe-tenant-a"));
    assert_eq!(rb["spawned"], json!("/tmp/ctx-probe-tenant-b"));
}

/// An explicit context wins over whatever the calling task happens to have
/// scoped: the caller said who the call is for.
#[tokio::test]
async fn explicit_context_overrides_the_calling_tasks_scope() {
    register_probe();
    let scoped = CoreContext::for_test(DomainSet::full(), Some("/tmp/ctx-probe-scoped".into()));
    let explicit = CoreContext::for_test(DomainSet::full(), Some("/tmp/ctx-probe-explicit".into()));
    let value = CoreContext::scope(scoped, async {
        try_invoke_registered_rpc(
            DispatchCtx::new(explicit),
            "openhuman.ext_ctx_probe_whoami",
            Map::new(),
        )
        .await
    })
    .await
    .expect("dispatchable")
    .expect("handler succeeds");
    assert_eq!(value["workspace"], json!("/tmp/ctx-probe-explicit"));
}

#[tokio::test]
async fn try_invoke_registered_rpc_routes_security_policy_info() {
    let workspace = tempfile::TempDir::new().expect("security policy workspace");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.action_dir = workspace.path().to_path_buf();
    config.config_path = workspace.path().join("config.toml");
    let ctx = CoreContext::for_test_with_config(DomainSet::full(), config);
    let out = try_invoke_registered_rpc(
        DispatchCtx::new(ctx),
        "openhuman.security_policy_info",
        Map::new(),
    )
    .await
    .expect("security policy info should be registered")
    .expect("security policy info should succeed");

    assert!(
        out.get("result").is_some() || out.get("autonomy").is_some(),
        "security policy info should return policy payload: {out}"
    );
}
