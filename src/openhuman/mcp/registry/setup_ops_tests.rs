//! Unit tests for the guided setup handlers.
//!
//! The handle parsing is what is testable without a running service, and it is
//! the part that matters: a handle the agent invented must be refused with a
//! message naming it, not silently looked up and missed.

use super::*;

#[test]
fn a_handle_map_is_parsed() {
    let parsed = parse_handles(HashMap::from([(
        "API_KEY".to_string(),
        "secret://abc123".to_string(),
    )]))
    .expect("a valid handle");

    assert_eq!(parsed["API_KEY"].as_str(), "secret://abc123");
}

#[test]
fn a_bare_handle_is_accepted() {
    // A caller may pass back either form.
    let parsed = parse_handles(HashMap::from([(
        "API_KEY".to_string(),
        "abc123".to_string(),
    )]))
    .expect("a bare handle");

    assert_eq!(parsed["API_KEY"].as_str(), "secret://abc123");
}

#[test]
fn an_invented_handle_is_refused_and_named() {
    let error = parse_handles(HashMap::from([(
        "API_KEY".to_string(),
        "not-a-handle".to_string(),
    )]))
    .expect_err("an invented handle");

    assert!(error.contains("not-a-handle"), "{error}");
}

#[test]
fn an_empty_handle_map_parses_to_nothing() {
    assert!(parse_handles(HashMap::new()).unwrap().is_empty());
}

// ── install_and_connect status reporting (#6110) ─────────────────────────────
//
// `mcp_setup_install_and_connect` declared a `status` discriminator it never
// emitted, and the `installed_disconnected` half of it was unobservable: the
// registry's `setup_install_and_connect` turns a failed connect into
// `Ok(ConnectOutcome { server_id, tools: vec![] })`, so the handler reported
// success — and published `McpServerConnected` — for a server that never came
// up. These pin the classification and the reply shape; the two functions are
// pure so neither needs a registry or a reachable MCP server.

use super::super::types::{ConnStatus, ServerStatus};

fn conn_status(status: ServerStatus, last_error: Option<&str>) -> ConnStatus {
    ConnStatus {
        server_id: "srv-1".to_string(),
        qualified_name: "acme/thing".to_string(),
        display_name: "Thing".to_string(),
        status,
        tool_count: 0,
        last_error: last_error.map(str::to_string),
        auth_hint: None,
    }
}

#[test]
fn a_connected_server_classifies_as_connected_with_no_error() {
    let entry = conn_status(ServerStatus::Connected, None);
    assert_eq!(
        super::classify_install_connect(Ok(Some(&entry))),
        ("connected", None)
    );
}

#[test]
fn a_connected_server_with_no_tools_is_still_connected() {
    // The state the old unconditional-success path existed to protect: a
    // server exposing only resources or prompts connects with zero tools.
    // `connected` reads from `ServerStatus`, never from the tool count.
    let mut entry = conn_status(ServerStatus::Connected, None);
    entry.tool_count = 0;
    assert_eq!(
        super::classify_install_connect(Ok(Some(&entry))).0,
        "connected"
    );
}

#[test]
fn a_failed_connect_reports_installed_disconnected_and_carries_the_reason() {
    let entry = conn_status(ServerStatus::Error, Some("dial tcp: connection refused"));
    let (status, error) = super::classify_install_connect(Ok(Some(&entry)));
    assert_eq!(status, "installed_disconnected");
    assert_eq!(error.as_deref(), Some("dial tcp: connection refused"));
}

#[test]
fn a_401_reports_installed_disconnected_rather_than_connected() {
    // `Unauthorized` is reachable, distinct from `Error`, and is emphatically
    // not connected — a caller offering a sign-in path needs to see that.
    let entry = conn_status(ServerStatus::Unauthorized, Some("server answered 401"));
    let (status, error) = super::classify_install_connect(Ok(Some(&entry)));
    assert_eq!(status, "installed_disconnected");
    assert_eq!(error.as_deref(), Some("server answered 401"));
}

#[test]
fn a_non_connected_status_without_a_reason_still_names_the_state() {
    // `last_error` is `Option`; the schema's `error` is documented as present
    // whenever `status != connected`, so it must not go missing here.
    let entry = conn_status(ServerStatus::Disconnected, None);
    let (status, error) = super::classify_install_connect(Ok(Some(&entry)));
    assert_eq!(status, "installed_disconnected");
    assert!(
        error.as_deref().is_some_and(|e| e.contains("disconnected")),
        "the reason must name the state it saw, got: {error:?}"
    );
}

#[test]
fn a_missing_status_row_is_not_assumed_connected() {
    let (status, error) = super::classify_install_connect(Ok(None));
    assert_eq!(
        status, "installed_disconnected",
        "a server with no status row must never be reported as connected"
    );
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("no status row")),
        "and it must say which of the two unknowns it hit, got: {error:?}"
    );
}

#[test]
fn a_failed_status_query_surfaces_its_own_reason() {
    // The status *method* failing is a different fact from the server being
    // disconnected, and the reason must reach the caller rather than only a
    // log line (tinysweeper on #6132).
    let (status, error) = super::classify_install_connect(Err("store is poisoned"));
    assert_eq!(status, "installed_disconnected");
    let error = error.expect("a failed read-back must carry a reason");
    assert!(
        error.contains("store is poisoned"),
        "the underlying error must be surfaced, not swallowed: {error}"
    );
    assert!(
        error.contains("could not be read back"),
        "and it must say the read-back is what failed: {error}"
    );
}

#[test]
fn the_connected_payload_carries_tools_and_no_error() {
    let payload =
        super::install_and_connect_payload("srv-1", "acme/thing", "connected", None, Vec::new());
    assert_eq!(
        payload.get("server_id").and_then(|v| v.as_str()),
        Some("srv-1")
    );
    assert_eq!(
        payload.get("qualified_name").and_then(|v| v.as_str()),
        Some("acme/thing")
    );
    assert_eq!(
        payload.get("status").and_then(|v| v.as_str()),
        Some("connected")
    );
    assert!(
        payload.get("tools").is_some(),
        "schema declares `tools` iff status == connected: {payload}"
    );
    assert!(
        payload.get("error").is_none(),
        "schema declares `error` iff status != connected: {payload}"
    );
}

#[test]
fn the_disconnected_payload_carries_the_error_and_omits_tools() {
    let payload = super::install_and_connect_payload(
        "srv-1",
        "acme/thing",
        "installed_disconnected",
        Some("connection refused".to_string()),
        Vec::new(),
    );
    assert_eq!(
        payload.get("status").and_then(|v| v.as_str()),
        Some("installed_disconnected")
    );
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("connection refused")
    );
    assert!(
        payload.get("tools").is_none(),
        "`tools` must be absent when the server did not connect: {payload}"
    );
    // `server_id` stays required in both arms — the install did happen, and it
    // is what a caller needs to retry or remove the half-finished server.
    assert_eq!(
        payload.get("server_id").and_then(|v| v.as_str()),
        Some("srv-1")
    );
}

#[test]
fn every_declared_output_name_is_one_the_handler_can_emit() {
    // The drift #6110 is about: the catalog promised four fields, the handler
    // emitted three others. Walk the declared set against both reply arms.
    let declared: Vec<&str> = super::super::schemas::setup_schemas("install_and_connect")
        .outputs
        .iter()
        .map(|field| field.name)
        .collect();
    let connected = super::install_and_connect_payload("s", "q", "connected", None, Vec::new());
    let disconnected = super::install_and_connect_payload(
        "s",
        "q",
        "installed_disconnected",
        Some("boom".to_string()),
        Vec::new(),
    );
    for name in ["server_id", "qualified_name", "status", "tools", "error"] {
        assert!(
            declared.contains(&name),
            "`{name}` is emitted but not declared: {declared:?}"
        );
    }
    for name in &declared {
        assert!(
            connected.get(*name).is_some() || disconnected.get(*name).is_some(),
            "`{name}` is declared but neither reply arm emits it"
        );
    }
}
