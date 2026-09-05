use super::*;

#[tokio::test]
async fn list_action_requests_sends_tenant_and_auth() {
    let requests: Requests = Default::default();
    let app = Router::new()
        .route(
            "/api/v1/action-requests",
            get(
                |State(requests): State<Requests>,
                 uri: axum::http::Uri,
                 headers: HeaderMap| async move {
                    requests.lock().unwrap().push(CapturedRequest {
                        method: Method::GET,
                        path_and_query: uri.path_and_query().unwrap().as_str().to_string(),
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        actor: headers
                            .get("x-actor-id")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        idempotency_key: None,
                        body: Value::Null,
                    });
                    axum::Json(json!({
                        "items": [sample_action_request_envelope()],
                        "count": 1
                    }))
                },
            ),
        )
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let outcome = list_action_requests(
        &config,
        ListActionRequestsRpcParams {
            tenant_id: None,
            approval_state: Some("pending".into()),
            execution_state: None,
            limit: Some(20),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.value.len(), 1);
    assert_eq!(outcome.value[0].id, TEST_ACTION_REQUEST_ID);

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, Method::GET);
    assert!(captured[0]
        .path_and_query
        .contains("tenant_id=20000000-0000-0000-0000-000000000001"));
    assert!(captured[0]
        .path_and_query
        .contains("approval_state=pending"));
    assert_eq!(
        captured[0].authorization.as_deref(),
        Some("Bearer svc-token")
    );
    assert_eq!(captured[0].actor.as_deref(), Some("operator-workbench"));
}

#[tokio::test]
async fn approve_action_request_sends_decision_body_and_idempotency() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/action-requests/{TEST_ACTION_REQUEST_ID}/approve");
    let app = Router::new()
        .route(
            &route,
            post(
                |State(requests): State<Requests>,
                 method: Method,
                 uri: axum::http::Uri,
                 headers: HeaderMap,
                 body: Bytes| async move {
                    let parsed_body = if body.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_slice(&body).unwrap()
                    };
                    requests.lock().unwrap().push(CapturedRequest {
                        method,
                        path_and_query: uri.path_and_query().unwrap().as_str().to_string(),
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        actor: headers
                            .get("x-actor-id")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        idempotency_key: headers
                            .get("idempotency-key")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        body: parsed_body,
                    });
                    axum::Json(sample_action_request_envelope())
                },
            ),
        )
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let outcome = approve_action_request(
        &config,
        ActionRequestDecisionRpcParams {
            action_request_id: TEST_ACTION_REQUEST_ID.into(),
            reason: "looks safe".into(),
            expected_row_version: 2,
            idempotency_key: "ar-approve-stable".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.value.id, TEST_ACTION_REQUEST_ID);

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, Method::POST);
    assert_eq!(
        captured[0].idempotency_key.as_deref(),
        Some("ar-approve-stable")
    );
    assert_eq!(captured[0].body["decided_by"]["type"], json!("user"));
    assert_eq!(
        captured[0].body["decided_by"]["id"],
        json!(TEST_ACTOR_USER_ID)
    );
    assert_eq!(captured[0].body["reason"], json!("looks safe"));
    assert_eq!(captured[0].body["expected_row_version"], json!(2));
}

#[tokio::test]
async fn get_action_request_sends_get_path() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/action-requests/{TEST_ACTION_REQUEST_ID}");
    let app = Router::new()
        .route(
            &route,
            get(
                |State(requests): State<Requests>,
                 uri: axum::http::Uri,
                 headers: HeaderMap| async move {
                    requests.lock().unwrap().push(CapturedRequest {
                        method: Method::GET,
                        path_and_query: uri.path_and_query().unwrap().as_str().to_string(),
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        actor: headers
                            .get("x-actor-id")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        idempotency_key: None,
                        body: Value::Null,
                    });
                    axum::Json(sample_action_request_envelope())
                },
            ),
        )
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let outcome = get_action_request(
        &config,
        GetActionRequestRpcParams {
            action_request_id: TEST_ACTION_REQUEST_ID.into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.value.id, TEST_ACTION_REQUEST_ID);

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, Method::GET);
    assert_eq!(
        captured[0].path_and_query,
        format!("/api/v1/action-requests/{TEST_ACTION_REQUEST_ID}")
    );
    assert_eq!(
        captured[0].authorization.as_deref(),
        Some("Bearer svc-token")
    );
}

#[tokio::test]
async fn reject_action_request_sends_decision_body_and_idempotency() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/action-requests/{TEST_ACTION_REQUEST_ID}/reject");
    let app = Router::new()
        .route(
            &route,
            post(
                |State(requests): State<Requests>,
                 method: Method,
                 uri: axum::http::Uri,
                 headers: HeaderMap,
                 body: Bytes| async move {
                    let parsed_body = if body.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_slice(&body).unwrap()
                    };
                    requests.lock().unwrap().push(CapturedRequest {
                        method,
                        path_and_query: uri.path_and_query().unwrap().as_str().to_string(),
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        actor: headers
                            .get("x-actor-id")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        idempotency_key: headers
                            .get("idempotency-key")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        body: parsed_body,
                    });
                    axum::Json(sample_action_request_envelope())
                },
            ),
        )
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let outcome = reject_action_request(
        &config,
        ActionRequestDecisionRpcParams {
            action_request_id: TEST_ACTION_REQUEST_ID.into(),
            reason: "too risky".into(),
            expected_row_version: 2,
            idempotency_key: "ar-reject-stable".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.value.id, TEST_ACTION_REQUEST_ID);

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, Method::POST);
    assert_eq!(
        captured[0].idempotency_key.as_deref(),
        Some("ar-reject-stable")
    );
    assert_eq!(captured[0].body["decided_by"]["type"], json!("user"));
    assert_eq!(
        captured[0].body["decided_by"]["id"],
        json!(TEST_ACTOR_USER_ID)
    );
    assert_eq!(captured[0].body["reason"], json!("too risky"));
    assert_eq!(captured[0].body["expected_row_version"], json!(2));
}

#[tokio::test]
async fn reject_requires_non_empty_reason() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, "http://127.0.0.1:1".into());
    let err = reject_action_request(
        &config,
        ActionRequestDecisionRpcParams {
            action_request_id: TEST_ACTION_REQUEST_ID.into(),
            reason: "   ".into(),
            expected_row_version: 1,
            idempotency_key: "ar-reject-blank-reason".into(),
        },
    )
    .await
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert!(structured.message.contains("reason is required"));
}

#[tokio::test]
async fn reject_requires_non_empty_idempotency_key() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, "http://127.0.0.1:1".into());
    let err = reject_action_request(
        &config,
        ActionRequestDecisionRpcParams {
            action_request_id: TEST_ACTION_REQUEST_ID.into(),
            reason: "nope".into(),
            expected_row_version: 1,
            idempotency_key: "   ".into(),
        },
    )
    .await
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert!(structured.message.contains("idempotencyKey is required"));
    let data = structured.data.unwrap();
    assert_eq!(data["youpet"]["field"], json!("idempotencyKey"));
}

#[tokio::test]
async fn approve_decision_body_excludes_approver_class_and_decided_at() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/action-requests/{TEST_ACTION_REQUEST_ID}/approve");
    let app = Router::new()
        .route(
            &route,
            post(
                |State(requests): State<Requests>,
                 method: Method,
                 uri: axum::http::Uri,
                 headers: HeaderMap,
                 body: Bytes| async move {
                    let parsed_body = if body.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_slice(&body).unwrap()
                    };
                    requests.lock().unwrap().push(CapturedRequest {
                        method,
                        path_and_query: uri.path_and_query().unwrap().as_str().to_string(),
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        actor: headers
                            .get("x-actor-id")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        idempotency_key: headers
                            .get("idempotency-key")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        body: parsed_body,
                    });
                    axum::Json(sample_action_request_envelope())
                },
            ),
        )
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let _ = approve_action_request(
        &config,
        ActionRequestDecisionRpcParams {
            action_request_id: TEST_ACTION_REQUEST_ID.into(),
            reason: "exact body".into(),
            expected_row_version: 2,
            idempotency_key: "ar-approve-exact".into(),
        },
    )
    .await
    .unwrap();

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1);
    let body = &captured[0].body;
    let mut keys = body
        .as_object()
        .map(|m| m.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "decided_by".to_string(),
            "expected_row_version".to_string(),
            "reason".to_string(),
        ]
    );
    assert!(body.get("approver_class").is_none());
    assert!(body.get("decided_at").is_none());
    assert_eq!(
        body,
        &json!({
            "decided_by": { "type": "user", "id": TEST_ACTOR_USER_ID },
            "reason": "exact body",
            "expected_row_version": 2,
        })
    );
}

#[tokio::test]
async fn list_action_requests_forwards_all_query_params() {
    let requests: Requests = Default::default();
    let app = Router::new()
        .route(
            "/api/v1/action-requests",
            get(
                |State(requests): State<Requests>,
                 uri: axum::http::Uri,
                 headers: HeaderMap| async move {
                    requests.lock().unwrap().push(CapturedRequest {
                        method: Method::GET,
                        path_and_query: uri.path_and_query().unwrap().as_str().to_string(),
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        actor: headers
                            .get("x-actor-id")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        idempotency_key: None,
                        body: Value::Null,
                    });
                    axum::Json(json!({
                        "items": [sample_action_request_envelope()],
                        "count": 1
                    }))
                },
            ),
        )
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let _ = list_action_requests(
        &config,
        ListActionRequestsRpcParams {
            tenant_id: Some("20000000-0000-0000-0000-000000000001".into()),
            approval_state: Some("pending".into()),
            execution_state: Some("not_started".into()),
            limit: Some(25),
        },
    )
    .await
    .unwrap();

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1);
    let q = &captured[0].path_and_query;
    assert!(q.contains("tenant_id=20000000-0000-0000-0000-000000000001"));
    assert!(q.contains("approval_state=pending"));
    assert!(q.contains("execution_state=not_started"));
    assert!(q.contains("limit=25"));
}

#[tokio::test]
async fn list_action_requests_missing_tenant_is_config_error() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp, "http://127.0.0.1:1".into());
    config.youpet.tenant_id = None;
    let err = list_action_requests(
        &config,
        ListActionRequestsRpcParams {
            tenant_id: None,
            approval_state: None,
            execution_state: None,
            limit: None,
        },
    )
    .await
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    let data = structured.data.unwrap();
    assert_eq!(data["kind"], json!("YouPetConfigMissing"));
    assert_eq!(data["youpet"]["field"], json!("tenant_id"));
}
