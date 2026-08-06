//! Endpoint tests for the feedback board client, against a TCP stub that hands
//! back the raw request so the method line, query string, and body can be
//! asserted.

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::types::{FeedbackQuery, FeedbackSort, FeedbackStatus, FeedbackType};
use crate::openhuman::medulla::client::MedullaClient;

/// Accept one request, reply with `response`, and hand back the raw request.
async fn stub(response: Vec<u8>) -> (String, tokio::sync::oneshot::Receiver<String>) {
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

/// A 200 response carrying `data` in the standard success envelope.
fn ok(data: serde_json::Value) -> Vec<u8> {
    let body = json!({ "success": true, "data": data }).to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

/// A representative board row as the backend serializes it.
fn item(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "type": "bug",
        "title": "Crash on resume",
        "body": "It crashes.",
        "status": "open",
        "createdBy": "u1",
        "createdByName": "Ada",
        "upvoteCount": 4,
        "downvoteCount": 1,
        "score": 3,
        "rankScore": 1.5,
        "commentCount": 2,
        "github": { "issueNumber": 12, "issueUrl": "https://example.test/12" },
        "myVote": 1,
        "createdAt": "2026-07-19T00:00:00.000Z",
        "updatedAt": "2026-07-19T00:00:00.000Z"
    })
}

#[tokio::test]
async fn list_feedback_sends_filters_and_decodes_rows() {
    let (base, req) = stub(ok(json!({
        "items": [item("f1")],
        "total": 1,
        "page": 1,
        "limit": 50
    })))
    .await;
    let client = MedullaClient::new(base, "jwt-abc");
    let page = client
        .list_feedback(&FeedbackQuery {
            kind: Some(FeedbackType::Bug),
            status: Some(FeedbackStatus::Open),
            sort: FeedbackSort::Top,
            page: 1,
            limit: 50,
        })
        .await
        .unwrap();

    assert_eq!(page.total, 1);
    let row = &page.items[0];
    assert_eq!(row.id, "f1");
    assert_eq!(row.kind, FeedbackType::Bug);
    assert_eq!(row.status, FeedbackStatus::Open);
    assert_eq!(row.score, 3);
    assert_eq!(row.my_vote, 1);
    assert_eq!(row.created_by_name.as_deref(), Some("Ada"));
    assert_eq!(
        row.github.as_ref().unwrap().issue_url.as_deref(),
        Some("https://example.test/12")
    );

    let sent = req.await.unwrap();
    assert!(sent.starts_with("GET /feedback?"), "{sent}");
    assert!(sent.contains("sort=top"), "{sent}");
    assert!(sent.contains("type=bug"), "{sent}");
    assert!(sent.contains("status=open"), "{sent}");
    assert!(sent.contains("authorization: Bearer jwt-abc"), "{sent}");
}

#[tokio::test]
async fn list_feedback_omits_absent_filters_and_clamps_limit() {
    let (base, req) = stub(ok(
        json!({ "items": [], "total": 0, "page": 1, "limit": 100 }),
    ))
    .await;
    let client = MedullaClient::new(base, "jwt-abc");
    client
        .list_feedback(&FeedbackQuery {
            limit: 5000,
            ..Default::default()
        })
        .await
        .unwrap();

    let sent = req.await.unwrap();
    // The backend rejects limit > 100 with a 400, so the client clamps first.
    assert!(sent.contains("limit=100"), "{sent}");
    assert!(!sent.contains("type="), "{sent}");
    assert!(!sent.contains("status="), "{sent}");
    assert!(sent.contains("sort=hot"), "{sent}");
}

#[tokio::test]
async fn get_feedback_decodes_item_and_comments() {
    let (base, req) = stub(ok(json!({
        "feedback": item("f1"),
        "comments": [
            { "id": "c1", "user": "u2", "userName": "Grace", "body": "Same here",
              "createdAt": "2026-07-19T01:00:00.000Z" }
        ]
    })))
    .await;
    let client = MedullaClient::new(base, "jwt-abc");
    let detail = client.get_feedback("f1").await.unwrap();

    assert_eq!(detail.feedback.title, "Crash on resume");
    assert_eq!(detail.comments.len(), 1);
    assert_eq!(detail.comments[0].user_name.as_deref(), Some("Grace"));
    assert_eq!(detail.comments[0].body, "Same here");

    let sent = req.await.unwrap();
    assert!(sent.starts_with("GET /feedback/f1"), "{sent}");
}

#[tokio::test]
async fn vote_feedback_posts_value_and_returns_updated_item() {
    let (base, req) = stub(ok(item("f1"))).await;
    let client = MedullaClient::new(base, "jwt-abc");
    let updated = client.vote_feedback("f1", -1).await.unwrap();

    assert_eq!(updated.id, "f1");
    let sent = req.await.unwrap();
    assert!(sent.starts_with("POST /feedback/f1/vote"), "{sent}");
    assert!(sent.contains("\"value\":-1"), "{sent}");
}

#[tokio::test]
async fn vote_feedback_percent_encodes_the_id() {
    let (base, req) = stub(ok(item("f1"))).await;
    let client = MedullaClient::new(base, "jwt-abc");
    // An id containing a slash must not escape its path segment.
    let _ = client.vote_feedback("a/b", 0).await;

    let sent = req.await.unwrap();
    assert!(sent.starts_with("POST /feedback/a%2Fb/vote"), "{sent}");
}

#[tokio::test]
async fn comment_feedback_posts_body() {
    let (base, req) = stub(ok(json!({
        "id": "c9", "user": "u1", "userName": "Ada", "body": "Confirmed",
        "createdAt": "2026-07-19T02:00:00.000Z"
    })))
    .await;
    let client = MedullaClient::new(base, "jwt-abc");
    let comment = client.comment_feedback("f1", "Confirmed").await.unwrap();

    assert_eq!(comment.id, "c9");
    assert_eq!(comment.body, "Confirmed");
    let sent = req.await.unwrap();
    assert!(sent.starts_with("POST /feedback/f1/comments"), "{sent}");
    assert!(sent.contains("\"body\":\"Confirmed\""), "{sent}");
}

#[tokio::test]
async fn submit_feedback_tags_the_medulla_product() {
    let (base, req) = stub(ok(json!({
        "accepted": true, "reason": "ok", "feedback": item("f2")
    })))
    .await;
    let client = MedullaClient::new(base, "jwt-abc");
    let result = client
        .submit_feedback(FeedbackType::Feature, "Add X", "Please add X")
        .await
        .unwrap();

    assert!(result.accepted);
    assert_eq!(result.feedback.unwrap().id, "f2");

    let sent = req.await.unwrap();
    // Must go through /ingest with product=medulla, otherwise the backend files
    // the resulting GitHub issue into the backend repo instead of medulla's.
    assert!(sent.starts_with("POST /feedback/ingest"), "{sent}");
    assert!(sent.contains("\"product\":\"medulla\""), "{sent}");
    assert!(sent.contains("\"origin\":\"medulla-tui\""), "{sent}");
    assert!(sent.contains("\"type\":\"feature\""), "{sent}");
}

#[tokio::test]
async fn submit_feedback_surfaces_moderation_rejection_as_ok() {
    // A moderation rejection is HTTP 200 with accepted=false — not an error.
    let (base, _req) = stub(ok(json!({
        "accepted": false, "reason": "off-topic", "feedback": null
    })))
    .await;
    let client = MedullaClient::new(base, "jwt-abc");
    let result = client
        .submit_feedback(FeedbackType::Bug, "spam", "spam")
        .await
        .unwrap();

    assert!(!result.accepted);
    assert_eq!(result.reason, "off-topic");
    assert!(result.feedback.is_none());
}

#[tokio::test]
async fn unknown_type_and_status_decode_to_other() {
    let mut raw = item("f3");
    raw["type"] = json!("question");
    raw["status"] = json!("shelved");
    let (base, _req) = stub(ok(json!({ "items": [raw], "total": 1 }))).await;
    let client = MedullaClient::new(base, "jwt-abc");
    let page = client
        .list_feedback(&FeedbackQuery::default())
        .await
        .unwrap();

    assert_eq!(page.items[0].kind, FeedbackType::Other);
    assert_eq!(page.items[0].status, FeedbackStatus::Other);
}
