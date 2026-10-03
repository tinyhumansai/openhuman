use super::*;
use tinytools_std::filesystem::{ApplyPatchTool, CsvExportTool};

#[tokio::test]
async fn all_tools_executes_stock_and_twilio_family_against_fake_backend() {
    let backend = integration_test_support::spawn_fake_integration_backend().await;
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, &backend.base_url);
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&tmp, &cfg);

    let quote = find_tool(&tools, "stock_quote")
        .execute(serde_json::json!({ "symbol": "AAPL" }))
        .await
        .expect("stock_quote execute");
    assert!(quote.output().contains("AAPL"));
    assert!(quote.output().contains("latest trading day 2026-05-16"));

    let exchange = find_tool(&tools, "stock_exchange_rate")
        .execute(serde_json::json!({
            "from_currency": "BTC",
            "to_currency": "USD"
        }))
        .await
        .expect("stock_exchange_rate execute");
    assert!(exchange.output().contains("BTC/USD = 42.5"));

    let options = find_tool(&tools, "stock_options")
        .execute(serde_json::json!({
            "symbol": "AAPL",
            "require_greeks": true
        }))
        .await
        .expect("stock_options execute");
    assert!(options.output().contains("AAPL options chain"));
    assert!(options.output().contains("call 2026-06-19 @ 250"));

    let crypto = find_tool(&tools, "stock_crypto_series")
        .execute(serde_json::json!({
            "symbol": "BTC",
            "market": "USD",
            "limit": 2
        }))
        .await
        .expect("stock_crypto_series execute");
    assert!(crypto.output().contains("BTC/USD"));
    assert!(crypto.output().contains("2026-05-16"));

    let commodity = find_tool(&tools, "stock_commodity")
        .execute(serde_json::json!({
            "commodity": "WTI",
            "interval": "weekly",
            "limit": 2
        }))
        .await
        .expect("stock_commodity execute");
    assert!(commodity.output().contains("WTI (weekly)"));
    assert!(commodity.output().contains("2026-05-16  80.1000"));

    let twilio = find_tool(&tools, "twilio_call")
        .execute(serde_json::json!({
            "to": "+14155551234",
            "message": "Hello from tests"
        }))
        .await
        .expect("twilio_call execute");
    assert!(twilio.output().contains("Call SID: CA1234"));
    assert!(twilio.output().contains("Status: queued"));

    let requests = backend.requests();
    let paths: Vec<&str> = requests.iter().map(|req| req.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "/agent-integrations/financial-apis/quote",
            "/agent-integrations/financial-apis/exchange-rate",
            "/agent-integrations/financial-apis/options",
            "/agent-integrations/financial-apis/crypto-series",
            "/agent-integrations/financial-apis/commodity",
            "/agent-integrations/twilio/call",
        ]
    );
    assert_eq!(requests[2].body["requireGreeks"], serde_json::json!(true));
    assert_eq!(requests[5].body["to"], serde_json::json!("+14155551234"));
}

/// Every acting tool gates on `can_act()` and returns its own read-only refusal
/// string. Each of those must carry [`POLICY_BLOCKED_MARKER`] so the agent
/// harness recognizes the block as a hard reject and halts on a verbatim repeat
/// (see the marker detection in
/// `tinyagents::middleware::RepeatedToolFailureMiddleware`). This pins every tool's
/// literal to the marker const — drift between them fails here rather than
/// silently letting the agent grind on a doomed call. Args are the minimum
/// needed to reach the `can_act()` check in each tool.
#[tokio::test]
async fn readonly_acting_tools_carry_policy_blocked_marker() {
    use crate::security::{AutonomyLevel, POLICY_BLOCKED_MARKER};

    let tmp = TempDir::new().unwrap();
    let sec = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        workspace_dir: tmp.path().to_path_buf(),
        action_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });

    let cases: Vec<(Box<dyn Tool>, serde_json::Value)> = vec![
        (
            Box::new(ApplyPatchTool::new(sec.clone())),
            serde_json::json!({ "edits": [{ "path": "a.txt", "old_string": "x", "new_string": "y" }] }),
        ),
        (
            Box::new(CsvExportTool::new(sec.clone())),
            serde_json::json!({ "data": "col1\nval1", "filename": "x.csv" }),
        ),
        // The `computer`-family tools are compiled out with the
        // `desktop-automation` feature; gate these two cases per-element so the
        // rest of the read-only policy assertions still run in the slim build.
        #[cfg(feature = "modules")]
        (
            Box::new(BrowserOpenTool::new(
                sec.clone(),
                Arc::new(crate::modules::browser::BrowserClient::new(Arc::new(
                    crate::config::Config::default(),
                ))),
            )),
            serde_json::json!({ "url": "https://example.com" }),
        ),
        (
            Box::new(http_request_tool(sec.clone(), vec![], 0, 0)),
            serde_json::json!({ "url": "https://example.com" }),
        ),
    ];

    for (tool, args) in cases {
        let name = tool.name().to_string();
        let out = tool.execute(args).await.unwrap();
        assert!(out.is_error, "{name} should error under read-only autonomy");
        assert!(
            out.output().contains(POLICY_BLOCKED_MARKER),
            "{name} read-only block must carry {POLICY_BLOCKED_MARKER}, got: {}",
            out.output()
        );
    }
}

#[test]
fn productivity_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert_contains_all(&names, PRODUCTIVITY_TOOLS);
}

#[test]
fn productivity_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for off in PRODUCTIVITY_DEFAULT_OFF {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    for on in PRODUCTIVITY_ALWAYS_ON {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn productivity_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(
        &mut tools,
        &[
            "todo_destructive".to_string(),
            "task_source_manage".to_string(),
            "artifact_delete".to_string(),
        ],
    );
    let names = tool_names(&tools);
    for on in PRODUCTIVITY_DEFAULT_OFF {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
}

#[tokio::test]
async fn artifact_list_through_registry_returns_envelope() {
    let tmp = TempDir::new().unwrap();
    let tools = expansion_tools_for(&tmp);
    let out = find_tool(&tools, "artifact_list")
        .execute(serde_json::json!({ "limit": 10 }))
        .await
        .expect("artifact_list execute");
    let body = out.output_for_llm(false);
    assert!(body.contains("artifacts"), "envelope missing: {body}");
    assert!(body.contains("total"), "envelope missing total: {body}");
}

#[test]
fn knowledge_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));

    let mut expected_tools: Vec<&str> = Vec::new();

    // Add gated tools only when their feature is enabled. All of these —
    // list/describe/read_resource/recent_runs/read_run_log,
    // install_workflow_from_url, uninstall_workflow, and create_skill — are
    // registered under `#[cfg(feature = "skills")]` in ops.rs (skill/workflow
    // metadata + registry tools), not `flows`.
    if cfg!(feature = "skills") {
        expected_tools.extend(&[
            "list_workflows",
            "describe_workflow",
            "read_workflow_resource",
            "list_workflow_runs",
            "read_workflow_run_log",
            "install_workflow_from_url",
            "uninstall_workflow",
            "create_skill",
        ]);
    }

    assert_contains_all(&names, &expected_tools);

    // Verify that gated tools are absent when their feature is off
    if !cfg!(feature = "skills") {
        let skill_tools = [
            "create_skill",
            "list_workflows",
            "describe_workflow",
            "read_workflow_resource",
            "list_workflow_runs",
            "read_workflow_run_log",
            "install_workflow_from_url",
            "uninstall_workflow",
        ];
        for tool in &skill_tools {
            assert!(
                !names.iter().any(|n| n == tool),
                "{} should be absent when skills feature is off",
                tool
            );
        }
    }
}

#[test]
fn knowledge_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    let off_tools = knowledge_default_off();
    for off in &off_tools {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    let on_tools = knowledge_always_on();
    for on in &on_tools {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn knowledge_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["workflow_manage".to_string()]);
    let names = tool_names(&tools);
    let off_tools = knowledge_default_off();
    for on in &off_tools {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
}

#[test]
fn system_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert_contains_all(&names, SYSTEM_TOOLS);
}

#[test]
fn system_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for off in SYSTEM_DEFAULT_OFF {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    for on in SYSTEM_ALWAYS_ON {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn system_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["service_lifecycle".to_string()]);
    let names = tool_names(&tools);
    for on in SYSTEM_DEFAULT_OFF {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
}

#[tokio::test]
async fn health_system_info_through_registry() {
    let tmp = TempDir::new().unwrap();
    let tools = expansion_tools_for(&tmp);
    let out = find_tool(&tools, "health_system_info")
        .execute(serde_json::json!({}))
        .await
        .expect("health_system_info");
    assert!(out.output_for_llm(false).contains("os"));
}

#[test]
fn account_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert_contains_all(&names, ACCOUNT_TOOLS);
}

#[test]
fn account_tools_survive_a_narrow_user_preference_set() {
    // None of these is a user-toggleable family, so a preference snapshot that
    // names only `file_read` must leave every one of them advertised. This is
    // the assertion that would catch one of them being quietly added to
    // `TOOL_FAMILIES` as default-OFF.
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for on in ACCOUNT_TOOLS {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences; got: {names:?}"
        );
    }
}

#[test]
fn desktop_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    if !cfg!(feature = "modules") {
        assert!(!names.iter().any(|name| name.starts_with("desktop_")));
        return;
    }
    assert_contains_all(&names, DESKTOP_TOOLS);
    assert!(names.iter().any(|name| name == "desktop_goal"));
    assert!(names.iter().any(|name| name == "desktop_launch"));
    assert!(names.iter().any(|name| name == "desktop_snapshot"));
    assert!(
        !names.iter().any(|name| name == "desktop_act"),
        "raw ref actions must not enter the agent/tool_search registry"
    );
}

#[test]
fn desktop_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for off in DESKTOP_DEFAULT_OFF {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    for on in DESKTOP_ALWAYS_ON {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn desktop_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(
        &mut tools,
        &[
            "screen_permissions".to_string(),
            "mcp_manage".to_string(),
            "workspace_manage".to_string(),
        ],
    );
    let names = tool_names(&tools);
    for on in DESKTOP_DEFAULT_OFF {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
}
