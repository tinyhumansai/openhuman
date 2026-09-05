use super::*;

#[tokio::test]
async fn http_error_does_not_forward_core_response_body() {
    let app = Router::new().route(
        "/api/v1/workbench/alerts",
        get(|| async {
            (
                StatusCode::BAD_GATEWAY,
                axum::Json(json!({
                    "detail": {
                        "code": "core_failed",
                        "message": "sensitive upstream detail",
                        "internal_trace": "do-not-forward"
                    }
                })),
            )
        }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let err = list_alerts(&config, ListAlertsRpcParams::default())
        .await
        .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(
        structured.message,
        "YouPet Core request failed with HTTP 502"
    );
    let data = structured.data.unwrap();
    assert_eq!(data["kind"], json!("YouPetCoreHttpError"));
    assert_eq!(data["youpet"]["code"], json!("core_failed"));
    assert_eq!(data["youpet"]["http_status"], json!(502));
    assert!(
        !structured.expected_user_state,
        "5xx Core failures must remain reportable"
    );
    assert!(
        data["youpet"].get("response_body").is_none(),
        "Core response body must not cross the renderer boundary"
    );
    assert!(!data.to_string().contains("do-not-forward"));
    assert!(!data.to_string().contains("sensitive upstream detail"));
}

#[tokio::test]
async fn http_error_preserves_status_for_non_json_5xx_body() {
    let app = Router::new().route(
        "/api/v1/workbench/alerts",
        get(|| async {
            (
                StatusCode::BAD_GATEWAY,
                axum::response::Html("<html>proxy failed: do-not-forward-html</html>"),
            )
        }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let err = list_alerts(&config, ListAlertsRpcParams::default())
        .await
        .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(
        structured.message,
        "YouPet Core request failed with HTTP 502"
    );
    assert!(
        !structured.expected_user_state,
        "5xx Core failures must remain reportable"
    );
    let data = structured.data.unwrap();
    assert_eq!(data["kind"], json!("YouPetCoreHttpError"));
    assert_eq!(data["youpet"]["code"], json!("youpet_core_error"));
    assert_eq!(data["youpet"]["http_status"], json!(502));
    assert!(
        data["youpet"].get("response_body").is_none(),
        "Core response body must not cross the renderer boundary"
    );
    let data_string = data.to_string();
    assert!(!data_string.contains("do-not-forward-html"));
    assert!(!data_string.contains("parse_error"));
    assert!(!data_string.contains("YouPetCoreInvalidJson"));
}

#[tokio::test]
async fn http_error_marks_fastapi_validation_detail_array_as_expected_user_state() {
    let route = format!("/api/v1/alerts/{TEST_ALERT_ID}/ack");
    let app = Router::new().route(
        &route,
        post(|| async {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(json!({
                    "detail": [{
                        "type": "uuid_parsing",
                        "loc": ["body", "actor_user_id"],
                        "msg": "Input should be a valid UUID",
                        "input": "not-a-uuid-secret"
                    }]
                })),
            )
        }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let err = ack_alert(
        &config,
        AlertActionRpcParams {
            alert_id: TEST_ALERT_ID.into(),
            note: None,
            resolution: None,
            idempotency_key: Some("idem".into()),
        },
    )
    .await
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(
        structured.message,
        "YouPet Core request failed with HTTP 422"
    );
    assert!(
        structured.expected_user_state,
        "4xx Core failures should be treated as expected user state"
    );
    let data = structured.data.unwrap();
    assert_eq!(data["kind"], json!("YouPetCoreHttpError"));
    assert_eq!(data["youpet"]["code"], json!("youpet_core_error"));
    assert_eq!(data["youpet"]["http_status"], json!(422));
    assert!(
        data["youpet"].get("response_body").is_none(),
        "Core response body must not cross the renderer boundary"
    );
    let data_string = data.to_string();
    assert!(!data_string.contains("not-a-uuid-secret"));
    assert!(!data_string.contains("Input should be a valid UUID"));
    assert!(!data_string.contains("actor_user_id"));
}

#[tokio::test]
async fn http_error_marks_invalid_operator_reference_as_expected_user_state() {
    let route = format!("/api/v1/alerts/{TEST_ALERT_ID}/ack");
    let app = Router::new().route(
        &route,
        post(|| async {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(json!({
                    "detail": {
                        "code": "invalid_reference",
                        "field": "actor_user_id",
                        "message": "unknown user"
                    }
                })),
            )
        }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let err = ack_alert(
        &config,
        AlertActionRpcParams {
            alert_id: TEST_ALERT_ID.into(),
            note: None,
            resolution: None,
            idempotency_key: Some("idem".into()),
        },
    )
    .await
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(
        structured.message,
        "YouPet Core request failed with HTTP 422"
    );
    assert!(
        structured.expected_user_state,
        "Core invalid_reference should be expected config/user state"
    );
    let data = structured.data.unwrap();
    assert_eq!(data["kind"], json!("YouPetCoreHttpError"));
    assert_eq!(data["youpet"]["code"], json!("invalid_reference"));
    assert_eq!(data["youpet"]["http_status"], json!(422));
    assert!(
        data["youpet"].get("response_body").is_none(),
        "Core response body must not cross the renderer boundary"
    );
    assert!(!data.to_string().contains("unknown user"));
}

#[tokio::test]
async fn success_non_json_body_is_structured_invalid_json() {
    let app = Router::new().route("/api/v1/workbench/alerts", get(|| async { "not-json" }));
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let err = list_alerts(&config, ListAlertsRpcParams::default())
        .await
        .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(structured.message, "YouPet Core returned invalid JSON");
    assert_eq!(
        structured.data.unwrap()["kind"],
        json!("YouPetCoreInvalidJson")
    );
}

#[tokio::test]
async fn response_shape_violation_is_structured_error() {
    let app = Router::new().route(
        "/api/v1/workbench/alerts",
        get(|| async { axum::Json(json!({ "items": [{ "id": "missing-required-fields" }] })) }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let err = list_alerts(&config, ListAlertsRpcParams::default())
        .await
        .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(structured.message, "YouPet Core response shape mismatch");
    assert_eq!(
        structured.data.unwrap()["kind"],
        json!("YouPetCoreResponseShape")
    );
}

#[tokio::test]
async fn missing_service_token_is_structured_error() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp, "http://127.0.0.1:1".into());
    config.youpet.service_token = None;

    let err = list_alerts(&config, ListAlertsRpcParams::default())
        .await
        .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(
        structured.data.unwrap()["kind"],
        json!("YouPetConfigMissing")
    );
}

#[tokio::test]
async fn missing_operator_user_id_is_expected_config_error_for_actions() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp, "http://127.0.0.1:1".into());
    config.youpet.operator_user_id = None;

    let err = ack_alert(
        &config,
        AlertActionRpcParams {
            alert_id: TEST_ALERT_ID.into(),
            note: None,
            resolution: None,
            idempotency_key: Some("idem".into()),
        },
    )
    .await
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(
        structured.message,
        "youpet.operator_user_id is required for YouPet Workbench actions"
    );
    assert!(
        structured.expected_user_state,
        "missing operator is a local config/user-state issue"
    );
    let data = structured.data.unwrap();
    assert_eq!(data["kind"], json!("YouPetConfigMissing"));
    assert_eq!(data["youpet"]["field"], json!("operator_user_id"));
}
