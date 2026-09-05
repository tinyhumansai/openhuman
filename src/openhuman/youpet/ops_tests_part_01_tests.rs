use super::*;

#[tokio::test]
async fn list_alerts_sends_auth_actor_and_empty_status_filter() {
    let requests: Requests = Default::default();
    let app = Router::new()
        .route(
            "/api/v1/workbench/alerts",
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
                        "items": [{
                            "id": TEST_ALERT_ID,
                            "alert_type": "missed_checkin",
                            "severity": "critical",
                            "related_type": "task_instance",
                            "related_id": "task-1",
                            "status": "open",
                            "created_at": "2026-06-01T00:00:00Z",
                            "context": null,
                            "unknown_future_field": true
                        }]
                    }))
                },
            ),
        )
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let outcome = list_alerts(
        &config,
        ListAlertsRpcParams {
            status: CoreAlertStatusFilter::All,
            severity: Some(CoreAlertSeverity::Critical),
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome.value[0].id, TEST_ALERT_ID);
    let request = requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.method, Method::GET);
    assert_eq!(
        request.path_and_query,
        "/api/v1/workbench/alerts?status=&severity=critical"
    );
    assert_eq!(request.authorization.as_deref(), Some("Bearer svc-token"));
    assert_eq!(request.actor.as_deref(), Some("operator-workbench"));
}

#[tokio::test]
async fn get_alert_trace_sends_auth_actor_and_no_action_body() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/workbench/alerts/{TEST_ALERT_ID}/trace");
    let app = Router::new()
        .route(&route, get(capture_trace))
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let outcome = get_alert_trace(
        &config,
        TraceAlertRpcParams {
            alert_id: TEST_ALERT_ID.into(),
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome.value.alert_id, TEST_ALERT_ID);
    assert_eq!(
        outcome
            .value
            .workflow
            .as_ref()
            .map(|workflow| workflow.id.as_str()),
        Some("plan-1")
    );
    assert_eq!(
        outcome.value.entries[0].id,
        "alert:11111111-1111-4111-8111-111111111111"
    );
    let request = requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.path_and_query, route);
    assert_eq!(request.authorization.as_deref(), Some("Bearer svc-token"));
    assert_eq!(request.actor.as_deref(), Some("operator-workbench"));
    assert_eq!(request.idempotency_key, None);
    assert_eq!(request.body, Value::Null);
}

#[tokio::test]
async fn trace_404_is_expected_user_state() {
    let route = format!("/api/v1/workbench/alerts/{TEST_ALERT_ID}/trace");
    let app = Router::new().route(
        &route,
        get(|| async {
            (
                StatusCode::NOT_FOUND,
                axum::Json(json!({
                    "detail": {
                        "code": "not_found",
                        "message": "secret missing alert body"
                    }
                })),
            )
        }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let err = get_alert_trace(
        &config,
        TraceAlertRpcParams {
            alert_id: TEST_ALERT_ID.into(),
        },
    )
    .await
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(
        structured.message,
        "YouPet Core request failed with HTTP 404"
    );
    assert!(
        structured.expected_user_state,
        "Core 404 trace lookup should surface as expected user/config state"
    );
    let data = structured.data.unwrap();
    assert_eq!(data["kind"], json!("YouPetCoreHttpError"));
    assert_eq!(data["youpet"]["code"], json!("not_found"));
    assert_eq!(data["youpet"]["http_status"], json!(404));
    assert!(!data.to_string().contains("secret missing alert body"));
}

#[tokio::test]
async fn ack_generates_idempotency_key_when_omitted() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/alerts/{TEST_ALERT_ID}/ack");
    let app = Router::new()
        .route(&route, post(capture))
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let outcome = ack_alert(
        &config,
        AlertActionRpcParams {
            alert_id: TEST_ALERT_ID.into(),
            note: Some("Calling owner.".into()),
            resolution: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome.value.status, CoreAlertStatus::Acknowledged);
    let request = requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.body["actor_user_id"], json!(TEST_ACTOR_USER_ID));
    assert_eq!(request.body["note"], json!("Calling owner."));
    let key = request.idempotency_key.expect("idempotency key");
    Uuid::parse_str(&key).expect("uuid v4 formatted idempotency key");
}

#[tokio::test]
async fn ack_omits_note_when_not_supplied() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/alerts/{TEST_ALERT_ID}/ack");
    let app = Router::new()
        .route(&route, post(capture))
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    ack_alert(
        &config,
        AlertActionRpcParams {
            alert_id: TEST_ALERT_ID.into(),
            note: None,
            resolution: None,
            idempotency_key: Some("idem".into()),
        },
    )
    .await
    .unwrap();

    let request = requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.body["actor_user_id"], json!(TEST_ACTOR_USER_ID));
    assert!(
        request.body.get("note").is_none(),
        "omitted note must not be serialized as JSON null"
    );
}

#[tokio::test]
async fn ack_blank_idempotency_keys_fall_back_to_fresh_uuid_headers() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/alerts/{TEST_ALERT_ID}/ack");
    let app = Router::new()
        .route(&route, post(capture))
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    for raw_key in ["", "   "] {
        ack_alert(
            &config,
            AlertActionRpcParams {
                alert_id: TEST_ALERT_ID.into(),
                note: None,
                resolution: None,
                idempotency_key: Some(raw_key.into()),
            },
        )
        .await
        .unwrap();
    }

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let empty_fallback = requests[0]
        .idempotency_key
        .as_deref()
        .expect("empty key fallback header");
    Uuid::parse_str(empty_fallback).expect("empty key fallback must be a UUID");
    let whitespace_fallback = requests[1]
        .idempotency_key
        .as_deref()
        .expect("whitespace key fallback header");
    Uuid::parse_str(whitespace_fallback).expect("whitespace key fallback must be a UUID");
    assert_ne!(
        empty_fallback, whitespace_fallback,
        "blank fallback keys must be generated per attempt"
    );
}

#[tokio::test]
async fn ack_trims_supplied_idempotency_key_before_sending() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/alerts/{TEST_ALERT_ID}/ack");
    let app = Router::new()
        .route(&route, post(capture))
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    ack_alert(
        &config,
        AlertActionRpcParams {
            alert_id: TEST_ALERT_ID.into(),
            note: None,
            resolution: None,
            idempotency_key: Some(" idem ".into()),
        },
    )
    .await
    .unwrap();

    let request = requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.idempotency_key.as_deref(), Some("idem"));
}

#[tokio::test]
async fn resolve_honors_supplied_idempotency_key() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/alerts/{TEST_ALERT_ID}/resolve");
    let app = Router::new()
        .route(&route, post(capture))
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    resolve_alert(
        &config,
        AlertActionRpcParams {
            alert_id: TEST_ALERT_ID.into(),
            note: None,
            resolution: Some("done".into()),
            idempotency_key: Some("idem-supplied".into()),
        },
    )
    .await
    .unwrap();

    let request = requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.body["resolution"], json!("done"));
    assert_eq!(request.idempotency_key.as_deref(), Some("idem-supplied"));
}

#[tokio::test]
async fn resolve_omits_resolution_when_not_supplied() {
    let requests: Requests = Default::default();
    let route = format!("/api/v1/alerts/{TEST_ALERT_ID}/resolve");
    let app = Router::new()
        .route(&route, post(capture))
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    resolve_alert(
        &config,
        AlertActionRpcParams {
            alert_id: TEST_ALERT_ID.into(),
            note: None,
            resolution: None,
            idempotency_key: Some("idem-supplied".into()),
        },
    )
    .await
    .unwrap();

    let request = requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.body["actor_user_id"], json!(TEST_ACTOR_USER_ID));
    assert!(
        request.body.get("resolution").is_none(),
        "omitted resolution must not be serialized as JSON null"
    );
}

#[test]
fn build_url_preserves_core_api_base_path_prefixes() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, "https://core.example.test/youpet".into());

    let url = build_url(&config, "/api/v1/workbench/alerts").unwrap();

    assert_eq!(
        url,
        "https://core.example.test/youpet/api/v1/workbench/alerts"
    );
}
