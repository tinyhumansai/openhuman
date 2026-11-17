use super::*;
use crate::agent::tinyagents::middleware::approval::ApprovalSecurityMiddleware;

// ── ApprovalSecurityMiddleware ──────────────────────────────────────────

#[test]
fn approval_external_effect_resolution_walks_the_tool_sets() {
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(FakeTool {
            name: "send_email",
            cap: None,
            external: true,
        }),
        Box::new(FakeTool {
            name: "read_file",
            cap: None,
            external: false,
        }),
    ]);
    let mw = ApprovalSecurityMiddleware::new(vec![tools]);
    assert!(mw.has_external_effect("send_email", &json!({})));
    assert!(!mw.has_external_effect("read_file", &json!({})));
    // Unknown tool defaults to no external effect (nothing to gate).
    assert!(!mw.has_external_effect("missing", &json!({})));
}

#[cfg(feature = "modules")]
#[test]
fn desktop_external_effect_keeps_metadata_but_skips_approval_when_disabled() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(desktop_external_effect_inner());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[cfg(feature = "modules")]
async fn desktop_external_effect_inner() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        "[desktop]\napprovals_enabled = false\n",
    )
    .unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", temp.path());
    }
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(crate::desktop::control::tools::DesktopTool::new(
            Arc::new(crate::config::Config::default()),
            crate::desktop::control::tools::DesktopToolKind::Goal,
        )),
        Box::new(FakeTool {
            name: "send_email",
            cap: None,
            external: true,
        }),
    ]);
    let middleware = ApprovalSecurityMiddleware::new(vec![tools]);
    assert!(middleware.has_external_effect("desktop_goal", &json!({})));
    assert!(
        !middleware
            .requires_approval(
                &crate::core::runtime::DispatchCtx::ambient(),
                "desktop_goal",
                &json!({})
            )
            .await
    );
    assert!(
        middleware
            .requires_approval(
                &crate::core::runtime::DispatchCtx::ambient(),
                "send_email",
                &json!({})
            )
            .await
    );
    match previous {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_WORKSPACE", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_WORKSPACE") },
    }
}

#[test]
fn approval_identity_scopes_composio_dispatcher_grants_to_one_action() {
    assert_eq!(
        approval_tool_name(
            "composio_execute",
            &json!({ "tool": "  GMAIL_SEND_EMAIL  " })
        ),
        "composio_execute:GMAIL_SEND_EMAIL"
    );
    assert_eq!(
        approval_tool_name("composio_execute", &json!({ "tool": "GMAIL_DELETE_EMAIL" })),
        "composio_execute:GMAIL_DELETE_EMAIL"
    );
    assert_eq!(
        approval_tool_name("composio_execute", &json!({})),
        "composio_execute:<invalid-action>"
    );
    assert_eq!(
        approval_tool_name("send_email", &json!({ "tool": "ignored" })),
        "send_email"
    );
}
