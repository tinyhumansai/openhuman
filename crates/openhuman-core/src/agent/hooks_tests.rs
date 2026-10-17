use super::*;

#[test]
fn sanitize_success_includes_char_count() {
    let out = sanitize_tool_output("hello world", "read_file", true);
    assert_eq!(out, "read_file: ok (11 chars)");
}

#[test]
fn sanitize_success_empty_output() {
    let out = sanitize_tool_output("", "write_file", true);
    assert_eq!(out, "write_file: ok (0 chars)");
}

#[test]
fn sanitize_failure_timeout() {
    let out = sanitize_tool_output("connection timeout after 30s", "http_request", false);
    assert_eq!(out, "http_request: failed (timeout)");
}

#[test]
fn sanitize_failure_not_found() {
    let out = sanitize_tool_output("no such file or directory", "read_file", false);
    assert_eq!(out, "read_file: failed (not_found)");
}

#[test]
fn sanitize_failure_not_found_variant() {
    let out = sanitize_tool_output("resource Not Found", "api_call", false);
    assert_eq!(out, "api_call: failed (not_found)");
}

#[test]
fn sanitize_failure_permission_denied() {
    let out = sanitize_tool_output("Permission denied", "exec", false);
    assert_eq!(out, "exec: failed (permission_denied)");
}

#[test]
fn sanitize_failure_connection_error() {
    let out = sanitize_tool_output("network unreachable", "fetch", false);
    assert_eq!(out, "fetch: failed (connection_error)");
}

#[test]
fn sanitize_failure_connection_variant() {
    let out = sanitize_tool_output("Connection refused", "fetch", false);
    assert_eq!(out, "fetch: failed (connection_error)");
}

#[test]
fn sanitize_failure_parse_error() {
    let out = sanitize_tool_output("invalid JSON syntax", "parse", false);
    assert_eq!(out, "parse: failed (parse_error)");
}

#[test]
fn sanitize_failure_parse_variant() {
    let out = sanitize_tool_output("failed to parse response", "api", false);
    assert_eq!(out, "api: failed (parse_error)");
}

#[test]
fn sanitize_failure_unknown_tool() {
    let out = sanitize_tool_output("unknown tool requested", "bad_tool", false);
    assert_eq!(out, "bad_tool: failed (unknown_tool)");
}

#[test]
fn sanitize_failure_generic_error() {
    let out = sanitize_tool_output("something went wrong", "tool", false);
    assert_eq!(out, "tool: failed (error)");
}

#[test]
fn turn_context_serde_roundtrip() {
    let ctx = TurnContext {
        user_message: "hello".into(),
        assistant_response: "hi".into(),
        tool_calls: vec![ToolCallRecord {
            name: "read".into(),
            arguments: serde_json::json!({"path": "/tmp"}),
            success: true,
            output_summary: "read: ok (100 chars)".into(),
            duration_ms: 42,
        }],
        turn_duration_ms: 500,
        session_id: Some("sess-1".into()),
        agent_id: Some("orchestrator".into()),
        entrypoint: Some("cli".into()),
        iteration_count: 2,
    };
    let json = serde_json::to_string(&ctx).unwrap();
    let back: TurnContext = serde_json::from_str(&json).unwrap();
    assert_eq!(back.user_message, "hello");
    assert_eq!(back.tool_calls.len(), 1);
    assert_eq!(back.tool_calls[0].name, "read");
    assert_eq!(back.iteration_count, 2);
}

#[tokio::test]
async fn fire_hooks_accepts_empty_hook_list() {
    let ctx = TurnContext {
        user_message: "x".into(),
        assistant_response: "y".into(),
        tool_calls: vec![],
        turn_duration_ms: 1,
        session_id: None,
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    };
    // Should not panic
    fire_hooks(&[], &crate::core::runtime::DispatchCtx::unbound(), ctx);
}

// ── Detached hooks keep the dispatch's CoreContext ────────────────────────────

/// A detached hook task must observe the same [`CoreContext`] as the dispatch
/// that fired it, handed over explicitly. A bare `tokio::spawn` loses the `CURRENT_CONTEXT`
/// task-local — under a scoped multi-tenant dispatch that meant the hook fell
/// back to the process default context, and any context-derived work behind
/// it read and wrote another tenant's workspace. `fire_hooks` now re-enters
/// the context it was given inside the spawned task; this pins that.
#[tokio::test]
async fn fired_hooks_observe_the_firing_dispatchs_core_context() {
    use crate::core::runtime::context::CoreContext;

    struct ContextProbe {
        seen: tokio::sync::Mutex<
            Option<tokio::sync::oneshot::Sender<Option<std::sync::Arc<CoreContext>>>>,
        >,
    }

    #[async_trait::async_trait]
    impl PostTurnHook for ContextProbe {
        fn name(&self) -> &str {
            "context-probe"
        }
        async fn on_turn_complete(&self, _ctx: &TurnContext) -> anyhow::Result<()> {
            if let Some(tx) = self.seen.lock().await.take() {
                let _ = tx.send(CoreContext::current());
            }
            Ok(())
        }
    }

    let tenant_ctx = CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(std::path::PathBuf::from("/tmp/tenant-a")),
    );
    let (tx, rx) = tokio::sync::oneshot::channel();
    let probe: std::sync::Arc<dyn PostTurnHook> = std::sync::Arc::new(ContextProbe {
        seen: tokio::sync::Mutex::new(Some(tx)),
    });
    let turn = TurnContext {
        user_message: String::new(),
        assistant_response: String::new(),
        tool_calls: Vec::new(),
        turn_duration_ms: 0,
        session_id: None,
        agent_id: None,
        entrypoint: None,
        iteration_count: 0,
    };

    // The explicit context is what the hook runs under; no task scope is
    // entered around the call.
    fire_hooks(
        &[probe],
        &crate::core::runtime::DispatchCtx::new(std::sync::Arc::clone(&tenant_ctx)),
        turn,
    );

    let seen = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .expect("hook must run")
        .expect("probe sender must fire");
    let seen = seen.expect("the hook must observe SOME ambient context");
    assert!(
        std::sync::Arc::ptr_eq(&seen, &tenant_ctx),
        "the detached hook must observe the dispatch's scoped context, \
         not the process default"
    );
}

/// The process-wide snapshot is independent of any task scope: a hook the host
/// registered is listed by name whatever context is active, and the per-context
/// snapshot starts from the same list.
#[tokio::test]
async fn embedder_snapshots_read_the_process_registry_not_the_task_scope() {
    use crate::core::runtime::context::CoreContext;

    struct Named;
    #[async_trait::async_trait]
    impl PostTurnHook for Named {
        fn name(&self) -> &str {
            "snapshot-scope-probe"
        }
        async fn on_turn_complete(&self, _ctx: &TurnContext) -> anyhow::Result<()> {
            Ok(())
        }
    }

    replace_embedder_post_turn_hook("snapshot-scope-probe", Some(std::sync::Arc::new(Named)));
    let tenant_ctx = CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let in_scope = CoreContext::scope(tenant_ctx, async {
        embedder_post_turn_hooks()
            .iter()
            .any(|hook| hook.name() == "snapshot-scope-probe")
    })
    .await;
    let via_ctx = embedder_post_turn_hooks_in(&crate::core::runtime::DispatchCtx::unbound())
        .iter()
        .any(|hook| hook.name() == "snapshot-scope-probe");
    replace_embedder_post_turn_hook("snapshot-scope-probe", None);

    assert!(in_scope, "the process registry is visible under a scope");
    assert!(
        via_ctx,
        "the per-context snapshot extends the same registry"
    );
}
