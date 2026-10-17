use super::*;

#[tokio::test]
async fn call_tool_records_write_argument_rejection() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let config = config_rpc::load_config_with_timeout()
        .await
        .expect("config");

    let err = call_tool(
        &crate::core::runtime::DispatchCtx::ambient(),
        "memory.learn",
        json!({ "kind": "fact" }),
        "mcp:test",
    )
    .await
    .expect_err("missing text should reject");
    assert!(
        err.message().contains("missing required argument `text`"),
        "got: {}",
        err.message()
    );

    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = crate::mcp::audit::list_writes(
            &config,
            &crate::mcp::audit::McpWriteListQuery::default(),
        )
        .await
        .expect("list writes");
        if rows.len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].tool_name, "memory.learn");
    assert_eq!(rows[0].client_info, "mcp:test");
    assert!(rows[0]
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains("missing required argument `text`"));
    assert!(rows[0].args_summary.get("text").is_none());

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn call_tool_rejects_v1_memory_tools_as_unknown() {
    for name in ["memory.search", "memory.store", "memory.note", "tree.tag"] {
        let err = call_tool(
            &crate::core::runtime::DispatchCtx::ambient(),
            name,
            json!({}),
            "mcp:test",
        )
        .await
        .expect_err("removed tool");
        assert!(err.message().contains("unknown MCP tool"), "{name}");
    }
}

#[tokio::test]
async fn call_tool_validates_memory_arguments_before_dispatch() {
    // A rejected write is audited against the ambient workspace. Without its
    // own workspace under the env lock this test wrote its `memory.forget` row
    // into whichever workspace a concurrent audit test had just pointed
    // `OPENHUMAN_WORKSPACE` at, making their row counts flaky.
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let err = call_tool(
        &crate::core::runtime::DispatchCtx::ambient(),
        "memory.forget",
        json!({ "ids": [] }),
        "mcp:test",
    )
    .await
    .expect_err("empty ids rejected before any RPC");
    assert!(err.message().contains("`ids`"), "got: {}", err.message());
    let err = call_tool(
        &crate::core::runtime::DispatchCtx::ambient(),
        "memory.recall",
        json!({ "question": "q", "filter": { "x": 1 } }),
        "mcp:test",
    )
    .await
    .expect_err("unknown filter field");
    assert!(err.message().contains("unexpected filter field"));
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

/// The policy a tool call is judged by comes from the context handed to
/// `call_tool`, not from whichever context the task happens to be scoped to,
/// and two sessions running side by side never see each other's.
#[tokio::test]
async fn call_tool_judges_the_policy_of_the_explicit_context_across_scopes() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::security::AutonomyLevel;
    use std::sync::Arc;

    let tmp_deny = tempfile::tempdir().expect("tempdir");
    let tmp_allow = tempfile::tempdir().expect("tempdir");
    let mut denying = crate::config::Config::default();
    denying.workspace_dir = tmp_deny.path().to_path_buf();
    denying.action_dir = tmp_deny.path().to_path_buf();
    denying.autonomy.enabled = true;
    denying.autonomy.level = AutonomyLevel::ReadOnly;
    let mut allowing = crate::config::Config::default();
    allowing.workspace_dir = tmp_allow.path().to_path_buf();
    allowing.action_dir = tmp_allow.path().to_path_buf();
    let deny_core = CoreContext::for_test_with_config(DomainSet::full(), denying);
    let allow_core = CoreContext::for_test_with_config(DomainSet::full(), allowing);

    let args = json!({ "agent_id": "no-such-agent", "prompt": "hello" });
    // Each call runs scoped to the *other* context, so an ambient read would
    // answer the wrong policy.
    let denied = CoreContext::scope(Arc::clone(&allow_core), {
        let ctx = DispatchCtx::new(Arc::clone(&deny_core));
        let args = args.clone();
        async move { call_tool(&ctx, "agent.run_subagent", args, "mcp:test").await }
    });
    let allowed = CoreContext::scope(Arc::clone(&deny_core), {
        let ctx = DispatchCtx::new(Arc::clone(&allow_core));
        async move { call_tool(&ctx, "agent.run_subagent", args, "mcp:test").await }
    });
    let (denied, allowed) = tokio::join!(denied, allowed);

    let denied = denied.expect_err("read-only context must refuse an act tool");
    let allowed = allowed.expect_err("unknown agent still fails, but not on policy");
    assert!(
        !denied.message().contains("no-such-agent"),
        "policy must refuse before the agent is built: {}",
        denied.message()
    );
    assert!(
        allowed.message().contains("no-such-agent"),
        "the permissive context reaches agent construction: {}",
        allowed.message()
    );
}
