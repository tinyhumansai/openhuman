//! Regression tests for multi-MCP-server support (#3196).
//!
//! These verify the three risks the issue calls out:
//! 1. Two servers exposing identically-named tools both reach the agent
//!    surface without collision (namespaced tool_id keys via server_id).
//! 2. One server failing to connect does not block another from running.
//! 3. Tool listing aggregates across all connected servers and tolerates
//!    a missing one.
//!
//! All tests are hermetic — they use the `test-mcp-stub` binary built
//! by Cargo (exposed via `CARGO_BIN_EXE_test-mcp-stub`) and a fresh
//! temp-dir workspace so they do not share SQLite state with each other
//! or with `mcp_registry_e2e.rs`.

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::mcp_registry::types::{CommandKind, InstalledServer, Transport};
use openhuman_core::openhuman::mcp_registry::{connections, ops, store};

// ── helpers ──────────────────────────────────────────────────────────────────

fn fresh_workspace_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

fn make_stub_server(qualified: &str) -> InstalledServer {
    let stub = env!("CARGO_BIN_EXE_test-mcp-stub");
    InstalledServer {
        server_id: format!("multi-{}", uuid::Uuid::new_v4()),
        qualified_name: qualified.to_string(),
        display_name: qualified.to_string(),
        description: None,
        icon_url: None,
        command_kind: CommandKind::Binary,
        command: stub.to_string(),
        args: Vec::new(),
        env_keys: Vec::new(),
        config: None,
        installed_at: 0,
        last_connected_at: None,
        transport: Transport::Stdio,
        enabled: true,
    }
}

// ── extract response text (mirrors mcp_registry_e2e.rs convention) ───────────

fn extract_echo_text(result: &serde_json::Value) -> &str {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
}

// ── Test 1: name collision ────────────────────────────────────────────────────

/// Two stub servers both expose the same tool name (`echo`). After connecting
/// both, `all_connected_tools` returns two entries — one per server — and both
/// respond correctly when called by `server_id`, proving the `server_id` is
/// the disambiguator for identically-named tools.
#[tokio::test]
async fn two_servers_same_tool_name_no_collision() {
    let (_tmp, cfg) = fresh_workspace_config();

    let server_a = make_stub_server("@multi-test/echo-a");
    let server_b = make_stub_server("@multi-test/echo-b");

    // Both server_ids must be distinct (sanity guard for the disambiguation
    // assertions below).
    assert_ne!(
        server_a.server_id, server_b.server_id,
        "two stub servers must have distinct server_ids"
    );

    store::insert_server(&cfg, &server_a).expect("insert server_a");
    store::insert_server(&cfg, &server_b).expect("insert server_b");

    // Connect both — each spawns its own subprocess.
    let tools_a = connections::connect(&cfg, &server_a)
        .await
        .expect("connect server_a");
    let tools_b = connections::connect(&cfg, &server_b)
        .await
        .expect("connect server_b");

    assert_eq!(tools_a.len(), 1, "server_a: stub advertises one tool");
    assert_eq!(tools_b.len(), 1, "server_b: stub advertises one tool");
    assert_eq!(tools_a[0].name, "echo", "server_a tool name is `echo`");
    assert_eq!(tools_b[0].name, "echo", "server_b tool name is `echo`");

    // Both servers are in the connected aggregate.
    let all_tools = connections::all_connected_tools().await;
    let a_tools: Vec<_> = all_tools
        .iter()
        .filter(|(sid, _, _)| sid == &server_a.server_id)
        .collect();
    let b_tools: Vec<_> = all_tools
        .iter()
        .filter(|(sid, _, _)| sid == &server_b.server_id)
        .collect();

    assert_eq!(
        a_tools.len(),
        1,
        "server_a contributes exactly one tool to the aggregate"
    );
    assert_eq!(
        b_tools.len(),
        1,
        "server_b contributes exactly one tool to the aggregate"
    );
    assert_eq!(
        a_tools[0].2.name, "echo",
        "server_a's aggregated tool is `echo`"
    );
    assert_eq!(
        b_tools[0].2.name, "echo",
        "server_b's aggregated tool is `echo`"
    );

    // Both are `connected` in all_status.
    let statuses = connections::all_status(&cfg).await;
    let find = |id: &str| {
        statuses
            .iter()
            .find(|s| s.server_id == id)
            .cloned()
            .unwrap_or_else(|| panic!("status entry missing for {id}"))
    };
    assert_eq!(find(&server_a.server_id).status.as_str(), "connected");
    assert_eq!(find(&server_a.server_id).tool_count, 1);
    assert_eq!(find(&server_b.server_id).status.as_str(), "connected");
    assert_eq!(find(&server_b.server_id).tool_count, 1);

    let _ = connections::disconnect(&server_a.server_id).await;
    let _ = connections::disconnect(&server_b.server_id).await;
}

// ── Test 2: routing ───────────────────────────────────────────────────────────

/// `call_tool` routes by `server_id` even with a shared name — each server
/// subprocess receives and echoes back the exact payload sent to that server_id.
/// Demonstrates the agent → core call path works across multiple servers without
/// cross-wiring.
#[tokio::test]
async fn tool_calls_route_to_the_correct_server() {
    let (_tmp, cfg) = fresh_workspace_config();

    let server_a = make_stub_server("@route-test/echo-a");
    let server_b = make_stub_server("@route-test/echo-b");

    store::insert_server(&cfg, &server_a).expect("insert server_a");
    store::insert_server(&cfg, &server_b).expect("insert server_b");

    connections::connect(&cfg, &server_a)
        .await
        .expect("connect server_a");
    connections::connect(&cfg, &server_b)
        .await
        .expect("connect server_b");

    // Send distinct payloads to each server; verify each echoes its own input.
    let result_a = connections::call_tool(
        &server_a.server_id,
        "echo",
        serde_json::json!({ "message": "payload-for-a" }),
    )
    .await
    .expect("call_tool on server_a should succeed");

    let result_b = connections::call_tool(
        &server_b.server_id,
        "echo",
        serde_json::json!({ "message": "payload-for-b" }),
    )
    .await
    .expect("call_tool on server_b should succeed");

    let text_a = extract_echo_text(&result_a);
    let text_b = extract_echo_text(&result_b);

    assert_eq!(
        text_a, "payload-for-a",
        "server_a must echo back exactly what was sent to it"
    );
    assert_eq!(
        text_b, "payload-for-b",
        "server_b must echo back exactly what was sent to it"
    );

    // Cross-verify: calling server_b's id echoes server_b's payload — not server_a's.
    assert_ne!(
        text_a, text_b,
        "two servers must not produce identical outputs for different inputs"
    );

    let _ = connections::disconnect(&server_a.server_id).await;
    let _ = connections::disconnect(&server_b.server_id).await;
}

// ── Test 3: failure isolation ─────────────────────────────────────────────────

/// One server failing to connect must not prevent another from connecting or
/// being usable in the same session. The bad server records an `error` status
/// with a `last_error` message; the good server is `connected` and responds.
#[tokio::test]
async fn failed_connect_does_not_block_healthy_peer() {
    let (_tmp, cfg) = fresh_workspace_config();

    // "bad" server: non-existent binary.
    let mut bad = make_stub_server("@isolation-test/bad");
    bad.command = "/this/path/does/not/exist/bad-mcp".to_string();

    // "good" server: real stub.
    let good = make_stub_server("@isolation-test/good");

    store::insert_server(&cfg, &bad).expect("insert bad server");
    store::insert_server(&cfg, &good).expect("insert good server");

    // Connecting the bad server must fail.
    let connect_err = connections::connect(&cfg, &bad)
        .await
        .expect_err("connect bad server should fail");
    assert!(
        !connect_err.to_string().is_empty(),
        "bad connect error must not be empty"
    );

    // Connecting the good server must succeed independently.
    let good_tools = connections::connect(&cfg, &good)
        .await
        .expect("connect good server must succeed regardless of the bad peer");
    assert_eq!(good_tools.len(), 1, "good server exposes one tool");

    // all_status: bad → error with last_error, good → connected.
    let statuses = connections::all_status(&cfg).await;
    let find = |id: &str| {
        statuses
            .iter()
            .find(|s| s.server_id == id)
            .cloned()
            .unwrap_or_else(|| panic!("status entry missing for {id}"))
    };

    let bad_status = find(&bad.server_id);
    assert_eq!(
        bad_status.status.as_str(),
        "error",
        "bad server must report `error`"
    );
    assert!(
        bad_status
            .last_error
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "bad server must have a non-empty last_error"
    );

    let good_status = find(&good.server_id);
    assert_eq!(
        good_status.status.as_str(),
        "connected",
        "good server must be `connected`"
    );
    assert!(
        good_status.last_error.is_none(),
        "good server must have no last_error"
    );

    // The good server is still callable after the peer's failure.
    let result = connections::call_tool(
        &good.server_id,
        "echo",
        serde_json::json!({ "message": "isolation-check" }),
    )
    .await
    .expect("call_tool on good server must succeed after peer failure");

    assert_eq!(
        extract_echo_text(&result),
        "isolation-check",
        "good server must echo input correctly after peer failure"
    );

    let _ = connections::disconnect(&good.server_id).await;
}

// ── Test 4: disabled enforcement ──────────────────────────────────────────────

/// A disabled server must not contribute tools to the agent surface and must
/// refuse explicit connect, even when a sibling is connected and active.
#[tokio::test]
async fn disabled_server_contributes_no_tools_to_agent_surface() {
    let (_tmp, cfg) = fresh_workspace_config();

    // "live" server: enabled, real stub.
    let live = make_stub_server("@disabled-test/live");

    // "quiet" server: disabled, also uses the real stub binary (so the
    // bogus-command failure path cannot mask the disabled enforcement check).
    let mut quiet = make_stub_server("@disabled-test/quiet");
    quiet.enabled = false;

    store::insert_server(&cfg, &live).expect("insert live server");
    store::insert_server(&cfg, &quiet).expect("insert quiet server");

    // Connect the live server.
    connections::connect(&cfg, &live)
        .await
        .expect("connect live server");

    // Attempting to connect the disabled server via ops must fail with
    // a clear "disabled" message (matches mcp_registry_e2e::connect_refuses_disabled_server).
    let disabled_err = ops::mcp_clients_connect(&cfg, quiet.server_id.clone())
        .await
        .expect_err("connect must reject a disabled server");
    assert!(
        disabled_err.to_lowercase().contains("disabled"),
        "error must mention 'disabled', got: {disabled_err}"
    );

    // all_status: live → connected, quiet → disabled.
    let statuses = connections::all_status(&cfg).await;
    let find = |id: &str| {
        statuses
            .iter()
            .find(|s| s.server_id == id)
            .cloned()
            .unwrap_or_else(|| panic!("status entry missing for {id}"))
    };

    assert_eq!(find(&live.server_id).status.as_str(), "connected");
    assert_eq!(find(&quiet.server_id).status.as_str(), "disabled");

    // The disabled server must not appear in all_connected_tools.
    let all_tools = connections::all_connected_tools().await;
    let quiet_tools: Vec<_> = all_tools
        .iter()
        .filter(|(sid, _, _)| sid == &quiet.server_id)
        .collect();
    assert!(
        quiet_tools.is_empty(),
        "disabled server must contribute zero tools to the agent surface; found: {quiet_tools:?}"
    );

    // The live server IS present and callable.
    let live_tools: Vec<_> = all_tools
        .iter()
        .filter(|(sid, _, _)| sid == &live.server_id)
        .collect();
    assert_eq!(
        live_tools.len(),
        1,
        "live server must contribute its tool even while disabled peer is present"
    );

    let result = connections::call_tool(
        &live.server_id,
        "echo",
        serde_json::json!({ "message": "live-while-quiet" }),
    )
    .await
    .expect("call_tool on live server must succeed");

    assert_eq!(
        extract_echo_text(&result),
        "live-while-quiet",
        "live server echoes correctly while disabled peer exists"
    );

    let _ = connections::disconnect(&live.server_id).await;
}
