//! End-to-end test for the MCP setup-agent flow.
//!
//! Exercises the ref machinery + install_and_connect path without going
//! through a real upstream registry — the test inserts an
//! `InstalledServer` row directly to stand in for what
//! `install_and_connect` would have synthesised from
//! `registry::registry_get`. The transport itself is the same
//! `test-mcp-stub` binary used by `mcp_registry_e2e.rs`.

use std::collections::HashMap;
use std::time::Duration;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::mcp_registry::setup::{self, SecretRef};

#[tokio::test]
async fn request_secret_blocks_until_submit_then_resolves() {
    // Caller mints + awaits in one task, fulfiller submits in another.
    // The exact API the setup_ops::request_secret handler uses.
    let (r, rx) = setup::mint_request("API_KEY").await;

    let r_for_submit = r.clone();
    let submit_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let submitted = setup::fulfill(&r_for_submit, "shh-secret".to_string()).await;
        assert!(submitted, "fulfill returns true on first submit");
    });

    // The await side: must not return before fulfill is called.
    setup::await_fulfillment(&r, rx)
        .await
        .expect("await_fulfillment completes once submit lands");
    submit_task.await.unwrap();

    // Resolve maps {KEY: ref} -> {KEY: value} without exposing value to
    // anywhere it shouldn't be.
    let mut refs = HashMap::new();
    refs.insert("API_KEY".to_string(), r.clone());
    let resolved = setup::resolve_refs(&refs).await.expect("resolves");
    assert_eq!(
        resolved,
        vec![("API_KEY".to_string(), "shh-secret".to_string())]
    );

    // The setup-agent contract: once install_and_connect persists the
    // values, the refs are gone.
    let _ = setup::consume_refs(&refs).await.expect("consumes");
    assert!(
        setup::resolve_refs(&refs).await.is_err(),
        "post-consume resolve fails"
    );
}

#[tokio::test]
async fn test_connection_against_stub_returns_tools() {
    use openhuman_core::openhuman::mcp_client::McpStdioClient;

    // Mirror what setup_ops::test_connection does end-to-end, minus the
    // registry::registry_get step (we don't want to hit a real upstream
    // from CI). The point of this test is the spawn + initialize +
    // list_tools + teardown lifecycle the setup agent relies on.
    let (r, _rx) = setup::mint_request("ECHO_TOKEN").await;
    assert!(setup::fulfill(&r, "ignored-by-stub".to_string()).await);

    let mut refs = HashMap::new();
    refs.insert("ECHO_TOKEN".to_string(), r);
    let env = setup::resolve_refs(&refs).await.expect("resolves");
    assert_eq!(env.len(), 1);

    let stub_path = env!("CARGO_BIN_EXE_test-mcp-stub");
    let cfg = Config::default();
    let identity = cfg.mcp_client.client_identity.clone();
    let client = McpStdioClient::new(stub_path.to_string(), Vec::new(), env, None, identity);

    client.initialize().await.expect("stub initialises");
    let tools = client.list_tools().await.expect("stub lists tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");

    client.close_session().await.expect("stub closes");
}

#[tokio::test]
async fn invalid_ref_id_rejected_by_submit_secret() {
    // The submit_secret handler parses the ref id via SecretRef::parse.
    // Validate the parser independently here so an upstream regression
    // doesn't silently re-admit unsafe inputs.
    assert!(SecretRef::parse("secret://abc123").is_some());
    assert!(SecretRef::parse("abc123").is_some());
    assert!(SecretRef::parse("secret://not-hex!!").is_none());
    assert!(SecretRef::parse("").is_none());
    assert!(SecretRef::parse("../../etc/passwd").is_none());
}
