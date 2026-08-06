//! End-to-end tests of the HTTP endpoint surface driven against a local TCP
//! stub, asserting on request lines/bodies and the decoded responses as well
//! as transport/decode error paths.

use super::{http_json, spawn_stub};
use crate::openhuman::medulla::client::*;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Integration: create session / error envelope round-trips
// ---------------------------------------------------------------------------

#[tokio::test]
async fn integration_create_session_round_trip() {
    let body = r#"{"success":true,"data":{"sessionId":"sess-123"}}"#;
    let base = spawn_stub(http_json("HTTP/1.1 201 Created", body)).await;
    let client = MedullaClient::new(base, "jwt-abc");
    let created = client.create_session(Some("hello")).await.unwrap();
    assert_eq!(created.session_id, "sess-123");
}

#[tokio::test]
async fn integration_error_envelope_maps() {
    let body = r#"{"success":false,"error":"nope","errorCode":"TOKEN_EXPIRED"}"#;
    let base = spawn_stub(http_json("HTTP/1.1 401 Unauthorized", body)).await;
    let client = MedullaClient::new(base, "jwt-abc");
    let err = client.team_usage().await.unwrap_err();
    assert!(err.is_token_expired());
}

// ---------------------------------------------------------------------------
// Integration: full endpoint surface against the TCP stub
// ---------------------------------------------------------------------------

/// Like [`spawn_stub`], but also hands back the raw request bytes the client
/// sent so tests can assert on the method line, query string, and body.
async fn spawn_stub_capture(response: Vec<u8>) -> (String, tokio::sync::oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        sock.write_all(&response).await.unwrap();
        sock.flush().await.unwrap();
        let _ = sock.shutdown().await;
        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
    });
    (format!("http://{addr}"), rx)
}

/// A loopback address with nothing listening (bound then immediately released),
/// so a connect attempt is refused.
async fn dead_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

fn ok_envelope(status: &str, data: serde_json::Value) -> Vec<u8> {
    http_json(
        status,
        &json!({ "success": true, "data": data }).to_string(),
    )
}

#[tokio::test]
async fn routing_strategy_get_and_put_round_trip_camel_case() {
    use crate::openhuman::medulla::client::RoutingStrategy;

    // GET decodes the camelCase strategy; an unknown value degrades to None.
    let (base, req) = spawn_stub_capture(ok_envelope(
        "HTTP/1.1 200 OK",
        json!({ "strategy": "memoryFirst" }),
    ))
    .await;
    let client = MedullaClient::new(base, "jwt-abc");
    let got = client.get_routing_strategy().await.unwrap();
    assert_eq!(got, Some(RoutingStrategy::MemoryFirst));
    let sent = req.await.unwrap();
    assert!(
        sent.starts_with("GET /medulla/v1/routing/strategy"),
        "{sent}"
    );

    // PUT sends `{ "strategy": <camelCase> }`.
    let (base, req) = spawn_stub_capture(ok_envelope(
        "HTTP/1.1 200 OK",
        json!({ "strategy": "cpuFirst" }),
    ))
    .await;
    let client = MedullaClient::new(base, "jwt-abc");
    client
        .set_routing_strategy(RoutingStrategy::CpuFirst)
        .await
        .unwrap();
    let sent = req.await.unwrap();
    assert!(
        sent.starts_with("PUT /medulla/v1/routing/strategy"),
        "{sent}"
    );
    assert!(sent.contains("\"strategy\":\"cpuFirst\""), "{sent}");
}

#[tokio::test]
async fn roster_decodes_capacity_and_safe_budgets() {
    let data = json!({
        "workers": [{
            "registryId": "agent-1",
            "label": "Reviewer",
            "description": "Reviews changes",
            "availability": "online",
            "harness": "codex",
            "cpuCores": 8,
            "selected": true,
            "budgets": [{
                "provider": "codex",
                "window": "weekly",
                "remainingTokens": 9000,
                "limitTokens": 10000,
                "source": "configured"
            }]
        }]
    });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let workers = client.roster().await.unwrap();
    assert_eq!(workers[0].registry_id, "agent-1");
    assert_eq!(workers[0].cpu_cores, Some(8));
    assert_eq!(workers[0].budgets[0].remaining_tokens, Some(9000));
    assert!(req.await.unwrap().starts_with("GET /medulla/v1/roster "));
}

#[tokio::test]
async fn program_task_create_uses_camel_case_contract() {
    let data = json!({
        "task": {
            "id": "task-1",
            "title": "Ship integration",
            "description": "",
            "status": "inProgress",
            "createdAt": "2026-07-25T00:00:00Z",
            "updatedAt": "2026-07-25T00:00:00Z",
            "dispatch": {}
        }
    });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 201 Created", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let task = client
        .create_program_task(CreateProgramTask {
            title: "Ship integration".into(),
            description: None,
            status: Some(ProgramTaskStatus::InProgress),
            recurrence: None,
        })
        .await
        .unwrap();
    assert_eq!(task.status, ProgramTaskStatus::InProgress);
    let sent = req.await.unwrap();
    assert!(sent.starts_with("POST /medulla/v1/tasks "), "{sent}");
    assert!(sent.contains("\"status\":\"inProgress\""), "{sent}");
}

#[test]
fn program_task_patch_distinguishes_omitted_and_cleared_recurrence() {
    let omitted = serde_json::to_value(UpdateProgramTask::default()).unwrap();
    assert_eq!(omitted, json!({}));

    let cleared = serde_json::to_value(UpdateProgramTask {
        recurrence: Some(None),
        ..UpdateProgramTask::default()
    })
    .unwrap();
    assert_eq!(cleared, json!({ "recurrence": null }));
}

#[tokio::test]
async fn program_resource_ids_are_encoded_as_single_path_segments() {
    let (base, req) =
        spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", json!({ "deleted": true }))).await;
    let client = MedullaClient::new(base, "jwt");
    assert!(client.delete_program_task("task/with?#").await.unwrap());
    assert!(req
        .await
        .unwrap()
        .starts_with("DELETE /medulla/v1/tasks/task%2Fwith%3F%23 "),);

    assert_eq!(
        super::super::urlencode("source/with?#"),
        "source%2Fwith%3F%23"
    );
}

#[tokio::test]
async fn program_source_sync_surfaces_nonfatal_provider_errors() {
    let data = json!({
        "result": {
            "added": 2,
            "updated": 1,
            "unchanged": 3,
            "errors": ["GitHub result truncated at the page cap"]
        }
    });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let result = client.sync_program_task_source("source-1").await.unwrap();
    assert_eq!(result.added, 2);
    assert_eq!(result.errors.len(), 1);
    assert!(req
        .await
        .unwrap()
        .starts_with("POST /medulla/v1/tasks/sources/source-1/sync "));
}

#[tokio::test]
async fn authed_get_carries_bearer_and_unwraps() {
    let (base, req) =
        spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", json!({ "spend": 1 }))).await;
    let client = MedullaClient::new(base, "jwt-abc");
    let usage = client.team_usage().await.unwrap();
    assert_eq!(usage["spend"], json!(1));
    let sent = req.await.unwrap();
    assert!(sent.contains("authorization: Bearer jwt-abc"), "{sent}");
}

#[tokio::test]
async fn team_usage_fetches_and_unwraps() {
    let data = json!({
        "plan": "pro",
        "inferenceTotals": { "spent": 1.5, "calls": 3 },
        "remainingUsd": 4.5,
    });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt-abc");
    let usage = client.team_usage().await.unwrap();
    assert_eq!(usage["plan"], json!("pro"));
    assert_eq!(usage["inferenceTotals"]["calls"], json!(3));
    let sent = req.await.unwrap();
    assert!(sent.starts_with("GET /teams/me/usage"), "{sent}");
    assert!(sent.contains("authorization: Bearer jwt-abc"), "{sent}");
}

#[tokio::test]
async fn list_sessions_decodes_rows() {
    let data = json!([
        { "sessionId": "s1", "title": "One", "status": "active", "lastSeq": 4 },
        { "sessionId": "s2", "status": "idle" },
    ]);
    let (base, _req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let rows = client.list_sessions().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].session_id, "s1");
    assert_eq!(rows[0].title.as_deref(), Some("One"));
    assert_eq!(rows[1].status, SessionStatus::Idle);
}

#[tokio::test]
async fn get_session_decodes_detail() {
    let data = json!({ "sessionId": "s9", "status": "active", "eventSeq": 12, "lastSeq": 12 });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let detail = client.get_session("s9").await.unwrap();
    assert_eq!(detail.session_id, "s9");
    assert_eq!(detail.event_seq, Some(12));
    let sent = req.await.unwrap();
    assert!(sent.starts_with("GET /medulla/v1/sessions/s9 "), "{sent}");
}

#[tokio::test]
async fn archive_session_round_trip() {
    let data = json!({ "sessionId": "s3", "status": "archived" });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let out = client.archive_session("s3").await.unwrap();
    assert_eq!(out.session_id, "s3");
    assert_eq!(out.status, SessionStatus::Archived);
    let sent = req.await.unwrap();
    assert!(sent.starts_with("DELETE /medulla/v1/sessions/s3"), "{sent}");
}

#[tokio::test]
async fn send_message_async_sets_sync_zero() {
    let data = json!({ "cycleId": "c1", "seq": 7 });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 202 Accepted", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let out = client.send_message("s1", "hello", false).await.unwrap();
    assert_eq!(out.cycle_id, "c1");
    assert_eq!(out.seq, 7);
    assert!(out.reply.is_none());
    let sent = req.await.unwrap();
    assert!(sent.contains("sync=0"), "{sent}");
    assert!(sent.contains("\"body\":\"hello\""), "{sent}");
}

#[tokio::test]
async fn send_message_sync_carries_reply_and_flag() {
    let data = json!({ "cycleId": "c2", "seq": 9, "reply": "done" });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let out = client.send_message("s1", "hi", true).await.unwrap();
    assert_eq!(out.reply.as_deref(), Some("done"));
    let sent = req.await.unwrap();
    assert!(sent.contains("sync=1"), "{sent}");
}

#[tokio::test]
async fn list_messages_passes_after_cursor() {
    let data = json!([{ "seq": 3, "role": "user", "body": "q", "ts": 100 }]);
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let msgs = client.list_messages("s1", Some(2)).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, Role::User);
    assert_eq!(msgs[0].seq, 3);
    let sent = req.await.unwrap();
    assert!(sent.contains("after=2"), "{sent}");
}

#[tokio::test]
async fn list_events_decodes_envelopes() {
    let data = json!([
        { "seq": 1, "at": 10, "sessionId": "s1", "event": { "kind": "assistant", "body": "hi" } },
    ]);
    let (base, _req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let evs = client.list_events("s1", None).await.unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind(), EventKind::Assistant { body: "hi".into() });
}

#[tokio::test]
async fn abort_round_trip() {
    let data = json!({ "sessionId": "s1", "aborted": true });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let out = client.abort("s1").await.unwrap();
    assert!(out.aborted);
    let sent = req.await.unwrap();
    assert!(
        sent.starts_with("POST /medulla/v1/sessions/s1/abort"),
        "{sent}"
    );
}

#[tokio::test]
async fn run_returns_reply_when_toolless() {
    let data = json!({ "reply": "hello", "passCount": 1, "sessionId": "s1", "cycleId": "c1" });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let out = client.run("go", RunOptions::default()).await.unwrap();
    match out {
        RunResult::Reply(r) => {
            assert_eq!(r.reply, "hello");
            assert_eq!(r.pass_count, Some(1));
        }
        other => panic!("expected reply, got {other:?}"),
    }
    let sent = req.await.unwrap();
    assert!(sent.starts_with("POST /orchestration/v1/run"), "{sent}");
    assert!(sent.contains("\"input\":\"go\""), "{sent}");
}

#[tokio::test]
async fn run_returns_loop_when_tools_present() {
    let data = json!({
        "stop": "tool_use",
        "cycleId": "c1",
        "sessionId": "s1",
        "toolCalls": [{ "id": "t1", "name": "search", "args": { "q": "x" } }],
    });
    let (base, _req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let opts = RunOptions {
        tools: Some(vec![ToolDef {
            name: "search".into(),
            description: "web".into(),
            parameters: json!({ "type": "object" }),
        }]),
        ..Default::default()
    };
    let out = client.run("go", opts).await.unwrap();
    match out {
        RunResult::Loop(LoopEvent::ToolUse { tool_calls, .. }) => {
            assert_eq!(tool_calls[0].name, "search");
        }
        other => panic!("expected tool_use loop, got {other:?}"),
    }
}

#[tokio::test]
async fn continue_run_decodes_loop_event() {
    let data = json!({ "stop": "end", "cycleId": "c1", "sessionId": "s1", "reply": "fin" });
    let (base, req) = spawn_stub_capture(ok_envelope("HTTP/1.1 200 OK", data)).await;
    let client = MedullaClient::new(base, "jwt");
    let results = vec![ToolResult {
        id: "t1".into(),
        ok: true,
        result: Some(json!({ "answer": 42 })),
        error: None,
    }];
    let ev = client.continue_run("c1", results).await.unwrap();
    match ev {
        LoopEvent::End { reply, .. } => assert_eq!(reply, "fin"),
        other => panic!("expected end, got {other:?}"),
    }
    let sent = req.await.unwrap();
    assert!(sent.contains("\"cycleId\":\"c1\""), "{sent}");
    assert!(sent.contains("\"toolResults\""), "{sent}");
}

#[tokio::test]
async fn http_error_without_envelope_becomes_api_error() {
    let (base, _req) = spawn_stub_capture(http_json(
        "HTTP/1.1 503 Service Unavailable",
        "upstream down",
    ))
    .await;
    let client = MedullaClient::new(base, "jwt");
    let err = client.get_session("s1").await.unwrap_err();
    assert_eq!(err.status(), Some(503));
    assert_eq!(err.error_code(), None);
    match err {
        ClientError::Api { message, .. } => assert_eq!(message, "upstream down"),
        other => panic!("expected api error, got {other:?}"),
    }
}

#[tokio::test]
async fn success_status_with_non_json_body_is_decode_error() {
    let (base, _req) = spawn_stub_capture(http_json("HTTP/1.1 200 OK", "not json at all")).await;
    let client = MedullaClient::new(base, "jwt");
    let err = client.team_usage().await.unwrap_err();
    assert!(matches!(err, ClientError::Decode(_)), "got {err:?}");
}

#[tokio::test]
async fn connection_refused_surfaces_transport_error() {
    let base = dead_addr().await;
    let client = MedullaClient::new(base, "jwt");
    let err = client.list_sessions().await.unwrap_err();
    assert!(matches!(err, ClientError::Transport(_)), "got {err:?}");
    assert_eq!(err.status(), None);
    assert_eq!(err.error_code(), None);
}
