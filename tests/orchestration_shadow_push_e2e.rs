//! Phase 0 shadow-migration coverage: the sanitized wire DTO (the security
//! allowlist gate) and the cloud event pusher's transport.
//!
//! These live in an integration crate rather than inline `#[cfg(test)]` modules
//! because the root crate's test build is currently blocked by unrelated stale
//! test modules at this checkout. An integration test links the *compiled* lib
//! (no `cfg(test)` siblings), so it runs cleanly here.

use std::time::Duration;

use openhuman_core::api::rest::BackendOAuthClient;
use openhuman_core::openhuman::orchestration::cloud::{push_event_with, push_world_diff_with};
use openhuman_core::openhuman::orchestration::wire::{
    parse_ts_ms, OrchestrationEventEnvelopeWire, WorldDiffBatchWire, WorldDiffEntryWire,
    ORCH_WIRE_PROTOCOL,
};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn envelope() -> OrchestrationEventEnvelopeWire {
    OrchestrationEventEnvelopeWire::build(
        "agent-alice",
        "sess-1",
        3,
        "user",
        "agent-alice",
        "hey there",
        1_700_000_000_000,
        "dm",
    )
}

// ── Wire DTO: the security allowlist gate ──────────────────────────────────

#[test]
fn wire_envelope_has_exactly_the_allowlisted_keys() {
    let value = envelope().to_value();
    let obj = value.as_object().expect("object");

    let mut top: Vec<&str> = obj.keys().map(String::as_str).collect();
    top.sort_unstable();
    assert_eq!(
        top,
        ["counterpartAgentId", "event", "protocol", "sessionId"]
    );

    let event = obj["event"].as_object().expect("event object");
    let mut event_keys: Vec<&str> = event.keys().map(String::as_str).collect();
    event_keys.sort_unstable();
    assert_eq!(event_keys, ["body", "kind", "role", "sender", "seq", "ts"]);

    assert_eq!(obj["protocol"], serde_json::json!(ORCH_WIRE_PROTOCOL));
}

#[test]
fn wire_envelope_never_carries_credential_or_path_keys() {
    let value = envelope().to_value();
    let obj = value.as_object().unwrap();
    let event = obj["event"].as_object().unwrap();
    for forbidden in [
        "path",
        "cwd",
        "workspace",
        "workspaceDir",
        "credentials",
        "token",
        "apiKey",
        "key",
        "signalKey",
        "identityKey",
        "ratchet",
        "secret",
    ] {
        assert!(!obj.contains_key(forbidden), "top-level leaked {forbidden}");
        assert!(!event.contains_key(forbidden), "event leaked {forbidden}");
    }
}

#[test]
fn wire_build_clamps_unknown_role_and_negative_scalars() {
    let env = OrchestrationEventEnvelopeWire::build("a", "s", -5, "robot", "a", "", -1, "");
    assert_eq!(env.event.role, "user");
    assert_eq!(env.event.seq, 0);
    assert_eq!(env.event.ts, 0);
    assert_eq!(env.event.kind, "message");
    assert_eq!(env.protocol, 1);
}

#[test]
fn wire_build_preserves_valid_roles() {
    for role in ["user", "assistant", "system"] {
        let env = OrchestrationEventEnvelopeWire::build("a", "s", 1, role, "a", "b", 1, "dm");
        assert_eq!(env.event.role, role);
    }
}

#[test]
fn parse_ts_ms_parses_rfc3339_and_rejects_garbage() {
    assert_eq!(parse_ts_ms("1970-01-01T00:00:01Z"), Some(1000));
    assert_eq!(parse_ts_ms("not-a-timestamp"), None);
}

// ── Cloud pusher transport ─────────────────────────────────────────────────

#[tokio::test]
async fn push_posts_sanitized_event_with_bearer_and_accepts_202() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orchestration/v1/events"))
        .and(header("authorization", "Bearer test-token"))
        .and(body_json(serde_json::json!({
            "protocol": 1,
            "counterpartAgentId": "agent-alice",
            "sessionId": "sess-1",
            "event": {
                "seq": 3,
                "role": "user",
                "sender": "agent-alice",
                "body": "hey there",
                "ts": 1_700_000_000_000i64,
                "kind": "dm"
            }
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
            "success": true,
            "data": { "accepted": true, "cycleId": "cyc:agent-alice:sess-1:3" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = BackendOAuthClient::new(&server.uri()).unwrap();
    let res = push_event_with(&client, "test-token", &envelope(), &[]).await;
    assert!(res.is_ok(), "expected ok, got {res:?}");
}

#[test]
fn world_diff_batch_has_exactly_the_allowlisted_keys() {
    let batch = WorldDiffBatchWire::build(
        "sess-1",
        vec![WorldDiffEntryWire::build(0, "peer online", 1)],
    );
    let value = batch.to_value();
    let obj = value.as_object().unwrap();
    let mut top: Vec<&str> = obj.keys().map(String::as_str).collect();
    top.sort_unstable();
    assert_eq!(top, ["entries", "protocol", "sessionId"]);

    let entry = obj["entries"][0].as_object().unwrap();
    let mut ek: Vec<&str> = entry.keys().map(String::as_str).collect();
    ek.sort_unstable();
    assert_eq!(ek, ["note", "seq", "ts"]);
    assert_eq!(obj["protocol"], serde_json::json!(ORCH_WIRE_PROTOCOL));
}

#[tokio::test]
async fn push_world_diff_posts_the_batch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orchestration/v1/world-diff"))
        .and(header("authorization", "Bearer test-token"))
        .and(body_json(serde_json::json!({
            "protocol": 1,
            "sessionId": "sess-1",
            "entries": [{ "seq": 0, "note": "peer online", "ts": 1i64 }]
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
            "success": true, "data": { "accepted": 1, "duplicates": 0, "tickScheduled": false }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = BackendOAuthClient::new(&server.uri()).unwrap();
    let batch = WorldDiffBatchWire::build(
        "sess-1",
        vec![WorldDiffEntryWire::build(0, "peer online", 1)],
    );
    let res = push_world_diff_with(&client, "test-token", &batch, &[]).await;
    assert!(res.is_ok(), "expected ok, got {res:?}");
}

#[tokio::test]
async fn push_returns_err_when_backend_5xxs_and_retries_exhausted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orchestration/v1/events"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = BackendOAuthClient::new(&server.uri()).unwrap();
    // No backoffs → single attempt, fails fast (no real sleeps).
    let res = push_event_with(&client, "test-token", &envelope(), &[]).await;
    assert!(res.is_err(), "expected err on 500");
}

#[tokio::test]
async fn push_retries_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orchestration/v1/events"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/orchestration/v1/events"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
            "success": true, "data": { "accepted": true, "cycleId": "c" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = BackendOAuthClient::new(&server.uri()).unwrap();
    // Zero-duration backoff → immediate retry, still exercises the loop.
    let res = push_event_with(&client, "test-token", &envelope(), &[Duration::ZERO]).await;
    assert!(res.is_ok(), "expected ok after retry, got {res:?}");
}
