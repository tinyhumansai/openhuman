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
    fire_hooks(&[], ctx);
}

// ── Detached hooks keep the dispatch's CoreContext ────────────────────────────

/// A detached hook task must observe the same ambient [`CoreContext`] as the
/// dispatch that fired it. A bare `tokio::spawn` loses the `CURRENT_CONTEXT`
/// task-local — under a scoped multi-tenant dispatch that meant the hook fell
/// back to the process default context, and the archivist/goals paths behind
/// it read and wrote another tenant's workspace. `fire_hooks` now re-enters
/// the captured scope inside the spawned task; this pins that.
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
        None,
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

    CoreContext::scope(std::sync::Arc::clone(&tenant_ctx), async {
        fire_hooks(&[probe], turn);
    })
    .await;

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
