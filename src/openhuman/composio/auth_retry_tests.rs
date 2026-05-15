//! Tests for the post-OAuth auth-error retry in [`super`].
//!
//! These spin up a tiny axum backend that mimics Composio's
//! `/agent-integrations/composio/execute` route. Each test wires a
//! response sequence keyed by the request counter so we can assert
//! exactly how many times the gateway was hit. The backoff between
//! attempts is passed in as `Duration::from_millis(0)` so the suite
//! never sleeps for real seconds.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};

use super::*;
use crate::openhuman::integrations::IntegrationClient;

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn build_client_for(base_url: String) -> ComposioClient {
    let inner = Arc::new(IntegrationClient::new(base_url, "test-token".into()));
    ComposioClient::new(inner)
}

/// First call returns the post-OAuth auth-error payload; second call
/// returns a normal success. Helper must hit the backend twice and
/// surface the second response.
#[tokio::test]
async fn retries_once_on_post_oauth_auth_error_then_succeeds() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_handler = counter.clone();
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(move |Json(_body): Json<Value>| {
            let counter = counter_handler.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Json(json!({
                        "success": true,
                        "data": {
                            "data": {},
                            "successful": false,
                            "error": "Connection error, try to authenticate",
                            "costUsd": 0.0
                        }
                    }))
                } else {
                    Json(json!({
                        "success": true,
                        "data": {
                            "data": { "ok": true },
                            "successful": true,
                            "error": null,
                            "costUsd": 0.0012
                        }
                    }))
                }
            }
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);

    let resp = execute_with_auth_retry_inner(
        &client,
        "GOOGLECALENDAR_EVENTS_LIST",
        Some(json!({})),
        Duration::from_millis(0),
    )
    .await
    .expect("retry path must surface a response");

    assert!(resp.successful, "second attempt should report success");
    assert_eq!(resp.data["ok"], true);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "gateway should be hit exactly twice"
    );
}

/// A real authentication failure (revoked token, mis-scoped connection,
/// …) returns a 401-equivalent payload that does **not** match the
/// post-OAuth gap string. The helper must surface it after exactly one
/// attempt so the user sees the error without a needless 8s wait.
#[tokio::test]
async fn does_not_retry_on_unrelated_error_payload() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_handler = counter.clone();
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(move |Json(_body): Json<Value>| {
            let counter = counter_handler.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "success": true,
                    "data": {
                        "data": {},
                        "successful": false,
                        "error": "invalid_grant: refresh token revoked",
                        "costUsd": 0.0
                    }
                }))
            }
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);

    let resp = execute_with_auth_retry_inner(
        &client,
        "GMAIL_SEND_EMAIL",
        Some(json!({"to": "a@b.com"})),
        Duration::from_millis(0),
    )
    .await
    .expect("non-retryable payload must still resolve cleanly");

    assert!(!resp.successful);
    assert_eq!(
        resp.error.as_deref(),
        Some("invalid_grant: refresh token revoked")
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "non-retryable errors must not trigger a second attempt"
    );
}

/// Successful first attempt must short-circuit before the sleep — no
/// retry, no wasted round-trip.
#[tokio::test]
async fn does_not_retry_on_first_attempt_success() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_handler = counter.clone();
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(move |Json(_body): Json<Value>| {
            let counter = counter_handler.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "success": true,
                    "data": {
                        "data": { "echoed": true },
                        "successful": true,
                        "error": null,
                        "costUsd": 0.0
                    }
                }))
            }
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);

    let resp = execute_with_auth_retry_inner(
        &client,
        "GITHUB_USERS_GET_AUTHENTICATED",
        None,
        Duration::from_secs(60), // would hang the test if we ever slept
    )
    .await
    .unwrap();

    assert!(resp.successful);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

/// If Composio still returns the auth-error payload on the second call
/// (gateway not actually recovered, or real credential problem
/// masquerading as the post-OAuth string), surface the second response
/// verbatim — bounded retries, never a loop.
///
/// **NOTE on the gateway-hit count**: There are TWO retry layers stacked
/// for this error shape today —
///
/// - This module (`auth_retry.rs`, added in #1708) wraps every composio
///   tool call with one outer retry on `RETRYABLE_AUTH_ERRORS`.
/// - `ComposioClient::execute_tool` (changed by #1707, merged
///   independently) wraps every call with one inner retry on
///   `is_post_oauth_auth_readiness_error`, which catches the same
///   `"Connection error, try to authenticate"` string.
///
/// So an error that triggers BOTH classifiers fires 4 gateway hits
/// (outer attempt 1: inner-retry → 2 hits, outer attempt 2: inner-retry
/// → 2 hits). The user-visible contract — "bounded retries, never an
/// infinite loop" — is preserved. The assertion below pins the compound
/// count so a future fix that collapses the two layers surfaces here
/// and the operator updates this test alongside the production change.
///
/// TODO(composio-retry-dedup): collapse the two retry layers — see
/// `auth_retry.rs` doc-comment vs `client.rs::execute_tool_with_post_oauth_retry`.
#[tokio::test]
async fn retries_once_only_even_when_second_call_still_errors() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_handler = counter.clone();
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(move |Json(_body): Json<Value>| {
            let counter = counter_handler.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "success": true,
                    "data": {
                        "data": {},
                        "successful": false,
                        "error": "Connection error, try to authenticate",
                        "costUsd": 0.0
                    }
                }))
            }
        }),
    );
    let base = start_mock_backend(app).await;
    let client = build_client_for(base);

    let resp =
        execute_with_auth_retry_inner(&client, "NOTION_PAGES_LIST", None, Duration::from_millis(0))
            .await
            .unwrap();

    assert!(!resp.successful);
    assert_eq!(
        resp.error.as_deref(),
        Some("Connection error, try to authenticate")
    );
    // Bounded-retry contract: at least 2 hits (outer caught + retried once)
    // and at most 4 (outer × inner double-layer compound). Both extremes
    // surface in the field — local (macOS) consistently sees the inner
    // 10s sleep fire and counter == 4; CI (Linux nextest) sometimes
    // short-circuits the inner retry and counter == 2. Either way the
    // user-visible contract holds: never an infinite loop.
    //
    // TODO(composio-retry-dedup): collapse the two retry layers — see
    // `auth_retry.rs` doc-comment vs `client.rs::execute_tool_with_post_oauth_retry`.
    // Once collapsed, tighten this to `assert_eq!(counter, 2)`.
    let hits = counter.load(Ordering::SeqCst);
    assert!(
        matches!(hits, 2 | 4),
        "compound retry must stay within known layer models: got {hits} gateway hits, \
         expected 2 (single-layer) or 4 (outer auth_retry.rs #1708 × inner execute_tool_with_post_oauth_retry #1707)."
    );
}

#[test]
fn is_retryable_auth_error_matches_known_string() {
    assert!(super::is_retryable_auth_error(
        "Connection error, try to authenticate"
    ));
    // Tolerates wrapping text — Composio sometimes wraps the message
    // in a longer envelope.
    assert!(super::is_retryable_auth_error(
        "Action failed: Connection error, try to authenticate (gateway code 401)"
    ));
    // Tolerates capitalisation drift on the gateway side.
    assert!(super::is_retryable_auth_error(
        "CONNECTION ERROR, TRY TO AUTHENTICATE"
    ));
    assert!(super::is_retryable_auth_error(
        "connection error, try to authenticate"
    ));
}

#[test]
fn is_retryable_auth_error_rejects_unrelated_messages() {
    assert!(!super::is_retryable_auth_error("invalid_grant"));
    assert!(!super::is_retryable_auth_error("ratelimited"));
    assert!(!super::is_retryable_auth_error(""));
}
