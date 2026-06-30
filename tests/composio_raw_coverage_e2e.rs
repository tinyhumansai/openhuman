//! Raw-line oriented coverage for deterministic Composio helpers.
//!
//! These tests avoid live Composio/backend calls and exercise public helper
//! surfaces that feed the JSON-RPC and agent-tool paths.

use std::sync::{Arc, Mutex};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::all::RegisteredController;
use openhuman_core::openhuman::composio::client::{
    create_composio_client, direct_execute, ComposioClientKind,
};
use openhuman_core::openhuman::composio::error_mapping::{
    classify_composio_error, format_provider_error, remap_transport_error, ComposioErrorClass,
};
use openhuman_core::openhuman::composio::execute_dispatch::{
    execute_composio_action, execute_composio_action_kind,
};
use openhuman_core::openhuman::composio::execute_prepare::prepare_execute_arguments;
use openhuman_core::openhuman::composio::oauth_handoff::{
    clear_non_active_connections, is_authorize_rate_limited, is_clearable_oauth_status,
    is_inflight_oauth_status, is_meta_oauth_toolkit, meta_oauth_rate_limit_message,
    wrap_authorize_rate_limit_error,
};
use openhuman_core::openhuman::composio::providers::{
    classify_unknown, find_curated, toolkit_from_slug, CuratedTool, ToolScope, UserScopePref,
};
use openhuman_core::openhuman::composio::tools::{
    ComposioAction, ComposioAuthorizeTool, ComposioConnectedAccount, ComposioExecuteTool,
    ComposioListConnectionsTool, ComposioListToolkitsTool, ComposioListToolsTool,
};
use openhuman_core::openhuman::composio::trigger_history::ComposioTriggerHistoryStore;
use openhuman_core::openhuman::composio::types::{
    ComposioActiveTrigger, ComposioActiveTriggersResponse, ComposioAgentReadyToolkitsResponse,
    ComposioAuthorizeResponse, ComposioAvailableTrigger, ComposioAvailableTriggerRepo,
    ComposioAvailableTriggersResponse, ComposioCapabilitiesResponse, ComposioCapability,
    ComposioConnection, ComposioConnectionsResponse, ComposioCreateTriggerResponse,
    ComposioDeleteResponse, ComposioDisableTriggerResponse, ComposioEnableTriggerResponse,
    ComposioExecuteResponse, ComposioGithubRepo, ComposioGithubReposResponse, ComposioToolFunction,
    ComposioToolSchema, ComposioToolkitsResponse, ComposioToolsResponse, ComposioTriggerEvent,
    ComposioTriggerHistoryEntry, ComposioTriggerHistoryResult, ComposioTriggerMetadata,
};
use openhuman_core::openhuman::composio::{
    all_composio_agent_tools, all_composio_controller_schemas, all_composio_registered_controllers,
    cached_active_integrations, connected_set_hash, connection_identity,
    fetch_connected_integrations, fetch_connected_integrations_status,
    init_composio_trigger_history, invalidate_connected_integrations_cache, ComposioActionTool,
    ComposioClient, FetchConnectedIntegrationsStatus,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::context::prompt::ConnectedIntegration;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::integrations::IntegrationClient;
use openhuman_core::openhuman::security::{AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::tools::{
    ComposioTool, PermissionLevel, Tool, ToolCallOptions, ToolCategory,
};

#[test]
fn composio_prepare_execute_arguments_normalizes_calendar_and_notion_payloads() {
    let calendar = prepare_execute_arguments(
        " GOOGLECALENDAR_EVENTS_LIST ",
        Some(json!({
            "timeMin": "2026-05-29",
            "time_max": "2026-05-30T15:00:00-07:00"
        })),
    )
    .expect("calendar args should normalize");
    assert_eq!(
        calendar.get("timeMin").and_then(Value::as_str),
        Some("2026-05-29T00:00:00Z")
    );
    assert_eq!(
        calendar.get("time_max").and_then(Value::as_str),
        Some("2026-05-30T15:00:00-07:00")
    );

    let invalid_date = prepare_execute_arguments(
        "GOOGLECALENDAR_FIND_EVENT",
        Some(json!({ "timeMax": "2026-99-99" })),
    )
    .expect_err("invalid bare dates should be rejected");
    assert!(invalid_date.contains("RFC 3339 timestamp"));

    let notion_pages = prepare_execute_arguments(
        "NOTION_FETCH_DATA",
        Some(json!({ "filter": { "value": "page" } })),
    )
    .expect("notion page filter should infer fetch type");
    assert_eq!(
        notion_pages.get("fetch_type").and_then(Value::as_str),
        Some("pages")
    );

    let notion_database = prepare_execute_arguments(
        "NOTION_FETCH_DATA",
        Some(json!({
            "fetchType": "databases",
            "filter": { "property": "page" }
        })),
    )
    .expect("explicit fetch type should win");
    assert_eq!(notion_database.get("fetch_type"), None);
    assert_eq!(
        notion_database.get("fetchType").and_then(Value::as_str),
        Some("databases")
    );
}

#[test]
fn composio_prepare_execute_arguments_validates_gmail_mutations() {
    let empty = prepare_execute_arguments("GMAIL_SEND_EMAIL", None)
        .expect_err("gmail send needs a recipient");
    assert!(empty.contains("recipient"));

    let send = prepare_execute_arguments(
        "GMAIL_SEND_EMAIL",
        Some(json!({ "recipientEmail": "person@example.test", "subject": "Hi" })),
    )
    .expect("recipientEmail alias should be accepted");
    assert_eq!(
        send.get("recipientEmail").and_then(Value::as_str),
        Some("person@example.test")
    );

    let missing_message = prepare_execute_arguments(
        "GMAIL_ADD_LABEL_TO_EMAIL",
        Some(json!({ "addLabelIds": ["Label_1"] })),
    )
    .expect_err("gmail add label needs a message id");
    assert!(missing_message.contains("message_id"));

    let missing_labels = prepare_execute_arguments(
        "GMAIL_ADD_LABEL_TO_EMAIL",
        Some(json!({ "messageId": "msg-1", "addLabelIds": ["  "] })),
    )
    .expect_err("gmail add label needs at least one non-empty label");
    assert!(missing_labels.contains("at least one"));

    let labeled = prepare_execute_arguments(
        "GMAIL_ADD_LABEL_TO_EMAIL",
        Some(json!({ "messageId": "msg-1", "remove_label_ids": "Label_2" })),
    )
    .expect("string label alias should be accepted");
    assert_eq!(
        labeled.get("messageId").and_then(Value::as_str),
        Some("msg-1")
    );

    let non_object = prepare_execute_arguments("GMAIL_SEND_EMAIL", Some(json!("bad")))
        .expect_err("arguments must be an object");
    assert!(non_object.contains("must be a JSON object"));
}

#[test]
fn composio_error_mapping_classifies_and_formats_provider_failures() {
    assert_eq!(ComposioErrorClass::Validation.as_str(), "validation");
    assert_eq!(
        ComposioErrorClass::InsufficientScope.as_str(),
        "insufficient_scope"
    );
    assert_eq!(ComposioErrorClass::RateLimited.as_str(), "rate_limited");
    assert_eq!(
        ComposioErrorClass::UpstreamProvider.as_str(),
        "upstream_provider"
    );
    assert_eq!(
        ComposioErrorClass::ComposioPlatform.as_str(),
        "composio_platform"
    );
    assert_eq!(ComposioErrorClass::Gateway.as_str(), "gateway");
    assert_eq!(ComposioErrorClass::Other.as_str(), "other");

    assert_eq!(
        classify_composio_error("GMAIL_SEND_EMAIL", "missing required field to"),
        ComposioErrorClass::Validation
    );
    assert_eq!(
        classify_composio_error(
            "GMAIL_FETCH_EMAILS",
            "403 insufficient authentication scopes for Gmail"
        ),
        ComposioErrorClass::InsufficientScope
    );
    assert_eq!(
        classify_composio_error("SLACK_POST_MESSAGE", "429 too many requests"),
        ComposioErrorClass::RateLimited
    );
    assert_eq!(
        classify_composio_error("GMAIL_FETCH_EMAILS", "Mailbox provider exploded"),
        ComposioErrorClass::UpstreamProvider
    );
    assert_eq!(
        classify_composio_error("CUSTOM_ACTION", "connection error, try to authenticate"),
        ComposioErrorClass::ComposioPlatform
    );
    assert_eq!(
        classify_composio_error("CUSTOM_ACTION", "502 Bad Gateway"),
        ComposioErrorClass::Gateway
    );
    assert_eq!(
        classify_composio_error("CUSTOM_ACTION", "plain unknown failure"),
        ComposioErrorClass::Other
    );

    let scope = format_provider_error(
        "GMAIL_FETCH_EMAILS",
        "insufficient authentication scopes: gmail.readonly",
    );
    assert!(scope.starts_with("[composio:error:insufficient_scope]"));
    assert!(scope.contains("Reconnect the integration"));

    let gateway = remap_transport_error(
        "GMAIL_FETCH_EMAILS",
        "Backend returned 502 Bad Gateway for POST: {\"error\":\"insufficient scope\"}",
    );
    assert!(
        gateway.starts_with("[composio:error:insufficient_scope]"),
        "embedded provider errors should not be bucketed as gateway: {gateway}"
    );

    let summarized_gateway = remap_transport_error(
        "CUSTOM_ACTION",
        "request failed: Backend returned 504 Gateway Timeout for POST /execute: edge timeout",
    );
    assert!(summarized_gateway.starts_with("[composio:error:gateway]"));
    assert!(summarized_gateway.contains("edge timeout"));

    let raw_gateway = remap_transport_error("CUSTOM_ACTION", "502 Bad Gateway");
    assert!(raw_gateway.contains("502 Bad Gateway"));

    let rate_limited = format_provider_error("SLACK_FETCH_CONVERSATION_HISTORY", "429");
    assert!(rate_limited.starts_with("[composio:error:rate_limited]"));
    assert!(rate_limited.contains("not an OpenHuman gateway outage"));

    let platform = format_provider_error("CUSTOM_ACTION", "token revoked");
    assert!(platform.starts_with("[composio:error:composio_platform]"));

    let validation_transport = remap_transport_error(
        "GMAIL_SEND_EMAIL",
        "Backend returned 502 Bad Gateway: missing required field `to`",
    );
    assert!(validation_transport.starts_with("[composio:error:validation]"));
}

#[test]
fn composio_oauth_handoff_helpers_classify_meta_status_and_rate_limits() {
    assert!(is_meta_oauth_toolkit(" Instagram "));
    assert!(is_meta_oauth_toolkit("FACEBOOK"));
    assert!(!is_meta_oauth_toolkit("gmail"));

    for status in ["pending", "INITIATED", " initializing "] {
        assert!(
            is_inflight_oauth_status(status),
            "{status} should be inflight"
        );
        assert!(
            is_clearable_oauth_status(status),
            "{status} should be clearable"
        );
    }
    for status in ["failed", "ERROR", " expired "] {
        assert!(!is_inflight_oauth_status(status));
        assert!(is_clearable_oauth_status(status));
    }
    assert!(!is_clearable_oauth_status("ACTIVE"));

    for message in ["HTTP 429", "too many requests", "rate_limit", "ratelimited"] {
        assert!(is_authorize_rate_limited(message));
    }
    assert!(!is_authorize_rate_limited("plain auth failure"));

    let instagram = meta_oauth_rate_limit_message("instagram");
    assert!(instagram.contains("Instagram Business or Creator"));
    let facebook = meta_oauth_rate_limit_message("facebook");
    assert!(facebook.contains("Business Manager"));
    let unknown = meta_oauth_rate_limit_message("threads");
    assert!(!unknown.contains("Business Manager"));

    let wrapped =
        wrap_authorize_rate_limit_error("instagram", anyhow::anyhow!("429 too many requests"));
    assert!(wrapped.to_string().contains("temporarily rate-limiting"));
    let passthrough = wrap_authorize_rate_limit_error("gmail", anyhow::anyhow!("429"));
    assert_eq!(passthrough.to_string(), "429");
}

#[tokio::test]
async fn composio_connected_integrations_public_helpers_handle_empty_auth_and_identity_edges() {
    let dir = tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };

    invalidate_connected_integrations_cache();
    assert!(cached_active_integrations(&config).is_none());

    let first = ConnectedIntegration {
        toolkit: "gmail".into(),
        description: "Gmail".into(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    };
    let second = ConnectedIntegration {
        toolkit: "slack".into(),
        description: "Slack".into(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    };
    let disconnected = ConnectedIntegration {
        toolkit: "notion".into(),
        description: "Notion".into(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: false,
        connections: Vec::new(),
        non_active_status: Some("EXPIRED".into()),
    };
    assert_eq!(
        connected_set_hash(&[first.clone(), second.clone(), disconnected.clone()]),
        connected_set_hash(&[disconnected, second, first])
    );
    assert_ne!(
        connected_set_hash(&[]),
        connected_set_hash(&[ConnectedIntegration {
            toolkit: "gmail".into(),
            description: String::new(),
            tools: Vec::new(),
            gated_tools: Vec::new(),
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        }])
    );

    let status = fetch_connected_integrations_status(&config).await;
    assert!(matches!(
        status,
        FetchConnectedIntegrationsStatus::Unavailable
    ));
    assert!(fetch_connected_integrations(&config).await.is_empty());
    assert!(cached_active_integrations(&config).is_none());

    assert_eq!(connection_identity(&config, "   ").await, None);
    assert_eq!(connection_identity(&config, "unknown-toolkit").await, None);
}

#[tokio::test]
async fn composio_ops_mode_and_trigger_history_are_local_and_deterministic() {
    let dir = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    config.composio.mode = "direct".into();

    let mode = openhuman_core::openhuman::composio::ops::composio_get_mode(&config)
        .await
        .expect("get mode should not call backend")
        .into_cli_compatible_json()
        .expect("mode outcome serializes");
    assert_eq!(mode.pointer("/result/mode"), Some(&json!("direct")));
    assert!(mode.pointer("/result/api_key_set").is_some());

    init_composio_trigger_history(dir.path().to_path_buf())
        .expect("global trigger history initializes for temp workspace");
    let store = openhuman_core::openhuman::composio::global_composio_trigger_history()
        .expect("global history store");
    store
        .record_trigger(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "metadata-local",
            "uuid-local",
            &json!({ "subject": "ops coverage" }),
        )
        .expect("record global trigger");

    let history =
        openhuman_core::openhuman::composio::ops::composio_list_trigger_history(&config, Some(0))
            .await
            .expect("history listing is local")
            .into_cli_compatible_json()
            .expect("history outcome serializes");
    assert_eq!(
        history.pointer("/result/entries/0/metadata_id"),
        Some(&json!("metadata-local"))
    );
    assert!(history
        .pointer("/result/archive_dir")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .contains("state/triggers"));
}

#[test]
fn composio_action_tool_metadata_is_stable_without_network_execution() {
    let dir = tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    let tool = ComposioActionTool::new(
        Arc::new(config),
        "GMAIL_SEND_EMAIL".into(),
        "Send an email".into(),
        Some(json!({
            "type": "object",
            "properties": { "to": { "type": "string" } }
        })),
    );

    assert_eq!(tool.name(), "GMAIL_SEND_EMAIL");
    assert_eq!(tool.description(), "Send an email");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert_eq!(
        tool.parameters_schema().pointer("/properties/to/type"),
        Some(&json!("string"))
    );

    let default_schema = ComposioActionTool::new(
        Arc::new(Config::default()),
        "NOTION_FETCH_DATA".into(),
        "Fetch Notion data".into(),
        None,
    );
    assert_eq!(
        default_schema.parameters_schema(),
        json!({ "type": "object" })
    );
}

#[tokio::test]
async fn composio_action_tool_execute_reports_factory_failures_without_network() {
    let tool = ComposioActionTool::new(
        Arc::new(Config::default()),
        "GMAIL_SEND_EMAIL".into(),
        "Send an email".into(),
        None,
    );

    let result = tool
        .execute(json!({ "subject": "missing recipient" }))
        .await
        .expect("local validation returns a tool result");
    assert!(result.is_error);
    let rendered = serde_json::to_string(&result).unwrap();
    assert!(rendered.contains("no backend session token"));
}

#[tokio::test]
async fn composio_client_and_dispatch_reject_invalid_inputs_before_network() {
    let inner = Arc::new(IntegrationClient::new(
        "http://127.0.0.1:0".into(),
        "test-token".into(),
    ));
    let client = ComposioClient::new(inner);
    let clone = client.clone();
    assert!(Arc::ptr_eq(client.inner(), clone.inner()));

    let auth_empty = client.authorize("   ", None).await.unwrap_err();
    assert!(auth_empty.to_string().contains("toolkit must not be empty"));
    let auth_non_object = client
        .authorize("whatsapp", Some(json!("waba-123")))
        .await
        .unwrap_err();
    assert!(auth_non_object
        .to_string()
        .contains("extra_params must be a JSON object"));
    let auth_reserved = client
        .authorize("whatsapp", Some(json!({ "client_id": "bad" })))
        .await
        .unwrap_err();
    assert!(auth_reserved
        .to_string()
        .contains("cannot override reserved key"));

    let delete_empty = client.delete_connection(" ").await.unwrap_err();
    assert!(delete_empty
        .to_string()
        .contains("connectionId must not be empty"));
    let execute_empty = client.execute_tool("\t", None).await.unwrap_err();
    assert!(execute_empty
        .to_string()
        .contains("tool slug must not be empty"));
    let create_empty = client.create_trigger(" ", None, None).await.unwrap_err();
    assert!(create_empty.to_string().contains("slug must not be empty"));
    let available_empty = client
        .list_available_triggers(" ", Some("conn-1"))
        .await
        .unwrap_err();
    assert!(available_empty
        .to_string()
        .contains("toolkit must not be empty"));
    let enable_missing_connection = client
        .enable_trigger(" ", "GMAIL_NEW_GMAIL_MESSAGE", None)
        .await
        .unwrap_err();
    assert!(enable_missing_connection
        .to_string()
        .contains("connectionId must not be empty"));
    let enable_missing_slug = client
        .enable_trigger("conn-1", " ", None)
        .await
        .unwrap_err();
    assert!(enable_missing_slug
        .to_string()
        .contains("slug must not be empty"));
    let disable_empty = client.disable_trigger("").await.unwrap_err();
    assert!(disable_empty
        .to_string()
        .contains("triggerId must not be empty"));

    let dispatch_empty = execute_composio_action(&client, " ", None)
        .await
        .unwrap_err();
    assert!(dispatch_empty.contains("tool slug must not be empty"));
    let dispatch_validation = execute_composio_action(
        &client,
        "GMAIL_SEND_EMAIL",
        Some(json!({ "subject": "missing recipient" })),
    )
    .await
    .unwrap_err();
    assert!(dispatch_validation.starts_with("[composio:error:"));
    assert!(dispatch_validation.contains("recipient"));

    let backend_kind = ComposioClientKind::Backend(client.clone());
    assert_eq!(backend_kind.mode(), "backend");
    let kind_empty = execute_composio_action_kind(backend_kind, " ", None, "entity")
        .await
        .unwrap_err();
    assert!(kind_empty.contains("tool slug must not be empty"));

    let kind_validation = execute_composio_action_kind(
        ComposioClientKind::Backend(client),
        "GMAIL_SEND_EMAIL",
        Some(json!({ "subject": "missing recipient" })),
        "entity",
    )
    .await
    .unwrap_err();
    assert!(kind_validation.starts_with("[composio:error:"));
    assert!(kind_validation.contains("recipient"));

    let direct_tool = Arc::new(ComposioTool::new(
        "direct-key",
        Some("entity-1"),
        Arc::new(SecurityPolicy::default()),
    ));
    let direct_kind = ComposioClientKind::Direct(direct_tool);
    assert_eq!(direct_kind.mode(), "direct");
    let direct_validation = execute_composio_action_kind(
        direct_kind,
        "GMAIL_SEND_EMAIL",
        Some(json!({ "subject": "still missing recipient" })),
        "entity-1",
    )
    .await
    .expect_err("direct dispatch validates before network");
    assert!(direct_validation.starts_with("[composio:error:"));
    assert!(direct_validation.contains("recipient"));
}

#[test]
fn composio_client_factory_modes_are_deterministic_without_network() {
    let dir = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };

    config.composio.mode = String::new();
    let backend_err = match create_composio_client(&config) {
        Ok(_) => panic!("backend without a session should fail"),
        Err(error) => error,
    };
    assert!(backend_err.to_string().contains("no backend session token"));

    config.composio.mode = "direct".into();
    let direct_err = match create_composio_client(&config) {
        Ok(_) => panic!("direct mode without an api key should fail"),
        Err(error) => error,
    };
    assert!(direct_err.to_string().contains("no api key is configured"));

    config.composio.api_key = Some("  cmp_test_key  ".into());
    let direct = create_composio_client(&config).expect("inline direct key builds a client");
    assert_eq!(direct.mode(), "direct");
    assert!(matches!(direct, ComposioClientKind::Direct(_)));

    config.composio.mode = "typo".into();
    let unknown = match create_composio_client(&config) {
        Ok(_) => panic!("unknown composio mode should fail"),
        Err(error) => error,
    };
    assert!(unknown.to_string().contains("unknown composio mode"));
}

#[tokio::test]
async fn composio_backend_client_local_validation_rejects_bad_inputs_before_http() {
    let client = ComposioClient::new(Arc::new(IntegrationClient::new(
        "http://127.0.0.1:9".to_string(),
        "unused-token".to_string(),
    )));

    let blank_authorize = client
        .authorize(" ", None)
        .await
        .expect_err("blank toolkit should fail before HTTP");
    assert!(blank_authorize
        .to_string()
        .contains("toolkit must not be empty"));

    let non_object_extra = client
        .authorize("gmail", Some(json!("bad")))
        .await
        .expect_err("extra params must be an object");
    assert!(non_object_extra
        .to_string()
        .contains("extra_params must be a JSON object"));

    let reserved_extra = client
        .authorize("gmail", Some(json!({ "toolkit": "slack" })))
        .await
        .expect_err("reserved keys cannot be overridden");
    assert!(reserved_extra
        .to_string()
        .contains("cannot override reserved key"));

    let blank_delete = client
        .delete_connection("\t")
        .await
        .expect_err("blank connection id should fail before HTTP");
    assert!(blank_delete.to_string().contains("connectionId"));

    let blank_execute = client
        .execute_tool(" ", Some(json!({})))
        .await
        .expect_err("blank tool should fail before HTTP");
    assert!(blank_execute.to_string().contains("tool slug"));

    let blank_create = client
        .create_trigger(" ", None, None)
        .await
        .expect_err("blank trigger slug should fail before HTTP");
    assert!(blank_create.to_string().contains("slug must not be empty"));

    let blank_available = client
        .list_available_triggers(" ", None)
        .await
        .expect_err("blank toolkit should fail before HTTP");
    assert!(blank_available.to_string().contains("toolkit"));

    let blank_enable_connection = client
        .enable_trigger(" ", "GMAIL_NEW_GMAIL_MESSAGE", None)
        .await
        .expect_err("blank connection id should fail before HTTP");
    assert!(blank_enable_connection.to_string().contains("connectionId"));

    let blank_enable_slug = client
        .enable_trigger("conn-1", " ", None)
        .await
        .expect_err("blank trigger slug should fail before HTTP");
    assert!(blank_enable_slug.to_string().contains("slug"));

    let blank_disable = client
        .disable_trigger(" ")
        .await
        .expect_err("blank trigger id should fail before HTTP");
    assert!(blank_disable.to_string().contains("triggerId"));

    let direct_tool = Arc::new(ComposioTool::new(
        "direct-api-key",
        Some("entity-1"),
        Arc::new(SecurityPolicy::default()),
    ));
    let blank_direct_execute = direct_execute(&direct_tool, " ", None, "entity-1", None)
        .await
        .expect_err("blank direct tool should fail before HTTP");
    assert!(blank_direct_execute.to_string().contains("tool slug"));
}

#[tokio::test]
async fn composio_backend_client_surfaces_get_post_envelope_and_status_errors() {
    async fn handler(request: Request) -> Response {
        let method = request.method().clone();
        let path = request.uri().path().to_string();
        match (method, path.as_str()) {
            (Method::GET, "/agent-integrations/composio/toolkits") => Json(json!({
                "success": false,
                "error": "Toolkit allowlist unavailable"
            }))
            .into_response(),
            (Method::GET, "/agent-integrations/composio/tools") => {
                Json(json!({ "success": true })).into_response()
            }
            (Method::POST, "/agent-integrations/composio/authorize") => {
                Json(json!({ "success": true })).into_response()
            }
            (Method::POST, "/agent-integrations/composio/execute") => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "success": false, "error": "upstream maintenance" })),
            )
                .into_response(),
            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({ "success": false, "error": format!("unhandled {path}") })),
            )
                .into_response(),
        }
    }

    let base = start_composio_round8_backend(Router::new().fallback(any(handler))).await;
    let client = ComposioClient::new(Arc::new(IntegrationClient::new(
        base,
        "round13-token".into(),
    )));

    let toolkits = client
        .list_toolkits()
        .await
        .expect_err("success=false GET envelopes should error");
    assert!(toolkits.to_string().contains("Backend error for GET"));
    assert!(toolkits
        .to_string()
        .contains("Toolkit allowlist unavailable"));

    let tools = client
        .list_tools(None, None)
        .await
        .expect_err("success=true without data should error");
    assert!(tools
        .to_string()
        .contains("Backend returned success but no data for GET"));

    let authorize = client
        .authorize("slack", None)
        .await
        .expect_err("POST success=true without data should error");
    assert!(authorize
        .to_string()
        .contains("Backend returned success but no data for POST"));

    let execute = client
        .execute_tool("SLACK_POST_MESSAGE", Some(json!({ "text": "hello" })))
        .await
        .expect_err("non-2xx POST should error");
    assert!(execute.to_string().contains("Backend returned 503"));
    assert!(execute.to_string().contains("upstream maintenance"));
}

#[tokio::test]
async fn composio_backend_factory_uses_stored_session_and_configured_backend() {
    async fn handler(request: Request) -> Response {
        let auth = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let method = request.method().clone();
        let path = request.uri().path().to_string();

        if auth != "Bearer stored-session-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "success": false, "error": format!("bad auth: {auth}") })),
            )
                .into_response();
        }

        match (method, path.as_str()) {
            (Method::GET, "/agent-integrations/composio/toolkits") => ok(json!({
                "toolkits": ["gmail"]
            })),
            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({ "success": false, "error": format!("unhandled {path}") })),
            )
                .into_response(),
        }
    }

    let base = start_composio_round8_backend(Router::new().fallback(any(handler))).await;
    let dir = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: dir.path().join("workspace"),
        config_path: dir.path().join("config.toml"),
        api_url: Some(base.clone()),
        ..Config::default()
    };
    config.composio.mode = "backend".into();
    store_app_session_token(&config, "  stored-session-token  ");

    let client = match create_composio_client(&config).expect("backend client from stored session")
    {
        ComposioClientKind::Backend(client) => client,
        ComposioClientKind::Direct(_) => panic!("backend mode should not create direct client"),
    };
    assert_eq!(client.inner().backend_url, base);

    let toolkits = client
        .list_toolkits()
        .await
        .expect("factory client should call local backend with stored bearer");
    assert_eq!(toolkits.toolkits, vec!["gmail"]);
}

#[tokio::test]
async fn composio_controller_registry_and_scope_handlers_cover_validation_edges() {
    let schemas = all_composio_controller_schemas();
    let registered = all_composio_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert!(schemas.iter().any(|schema| schema.function == "execute"));
    assert!(schemas
        .iter()
        .any(|schema| schema.function == "set_api_key"));
    assert!(registered.iter().all(|controller| {
        controller
            .rpc_method_name()
            .starts_with("openhuman.composio_")
    }));

    let unknown = openhuman_core::openhuman::composio::schemas::schemas("not_real");
    assert_eq!(unknown.function, "unknown");
    assert_eq!(unknown.inputs[0].name, "function");

    let get_scopes = composio_controller(&registered, "get_user_scopes");
    let scopes = composio_call(get_scopes, json!({ "toolkit": " Gmail " }))
        .await
        .expect("default user scopes");
    assert_eq!(scopes.pointer("/read"), Some(&json!(true)));
    assert_eq!(scopes.pointer("/write"), Some(&json!(true)));
    assert_eq!(scopes.pointer("/admin"), Some(&json!(false)));

    let missing_toolkit = composio_call(get_scopes, json!({}))
        .await
        .expect_err("toolkit is required");
    assert!(missing_toolkit.contains("missing required param 'toolkit'"));

    let set_scopes = composio_controller(&registered, "set_user_scopes");
    let invalid_write = composio_call(
        set_scopes,
        json!({ "toolkit": "gmail", "read": true, "write": "yes", "admin": false }),
    )
    .await
    .expect_err("write must be bool");
    assert!(invalid_write.contains("invalid 'write'"));
    let memory_missing = composio_call(
        set_scopes,
        json!({ "toolkit": "gmail", "read": true, "write": true, "admin": false }),
    )
    .await
    .expect_err("memory client not initialised");
    assert!(memory_missing.contains("memory client not initialised"));
}

#[test]
fn composio_controller_schema_catalog_covers_all_declared_functions() {
    let expected = [
        ("list_toolkits", 0, "toolkits"),
        ("list_capabilities", 0, "capabilities"),
        ("list_agent_ready_toolkits", 0, "toolkits"),
        ("list_connections", 0, "connections"),
        ("authorize", 2, "connectUrl"),
        ("delete_connection", 2, "deleted"),
        ("list_tools", 2, "tools"),
        ("execute", 3, "result"),
        ("list_github_repos", 1, "result"),
        ("create_trigger", 3, "result"),
        ("get_user_profile", 1, "profile"),
        ("refresh_all_identities", 0, "report"),
        ("sync", 2, "outcome"),
        ("list_trigger_history", 1, "result"),
        ("get_user_scopes", 1, "pref"),
        ("set_user_scopes", 4, "pref"),
        ("list_available_triggers", 2, "triggers"),
        ("list_triggers", 1, "triggers"),
        ("enable_trigger", 3, "result"),
        ("disable_trigger", 1, "deleted"),
        ("get_mode", 0, "mode"),
        ("set_api_key", 2, "result"),
        ("clear_api_key", 0, "result"),
    ];

    for (function, input_count, first_output) in expected {
        let schema = openhuman_core::openhuman::composio::schemas::schemas(function);
        assert_eq!(schema.namespace, "composio");
        assert_eq!(schema.function, function);
        assert_eq!(schema.inputs.len(), input_count, "{function}");
        assert_eq!(schema.outputs[0].name, first_output, "{function}");
        assert!(!schema.description.is_empty());
    }
}

#[tokio::test]
async fn composio_controller_handlers_reject_bad_params_before_network() {
    let registered = all_composio_registered_controllers();

    let missing_authorize = composio_call(composio_controller(&registered, "authorize"), json!({}))
        .await
        .expect_err("authorize requires toolkit");
    assert!(missing_authorize.contains("missing required param 'toolkit'"));

    let blank_delete = composio_call(
        composio_controller(&registered, "delete_connection"),
        json!({ "connection_id": " " }),
    )
    .await
    .expect_err("delete requires non-empty connection");
    assert!(blank_delete.contains("'connection_id' must not be empty"));

    let invalid_list_tools = composio_call(
        composio_controller(&registered, "list_tools"),
        json!({ "toolkits": "gmail" }),
    )
    .await
    .expect_err("toolkits must be an array");
    assert!(invalid_list_tools.contains("invalid 'toolkits'"));

    let missing_execute = composio_call(composio_controller(&registered, "execute"), json!({}))
        .await
        .expect_err("execute requires tool");
    assert!(missing_execute.contains("missing required param 'tool'"));

    let blank_create = composio_call(
        composio_controller(&registered, "create_trigger"),
        json!({ "slug": " " }),
    )
    .await
    .expect_err("create trigger rejects blank slug");
    assert!(blank_create.contains("'slug' must not be empty"));

    let missing_profile = composio_call(
        composio_controller(&registered, "get_user_profile"),
        json!({}),
    )
    .await
    .expect_err("profile requires connection id");
    assert!(missing_profile.contains("missing required param 'connection_id'"));

    let missing_sync = composio_call(composio_controller(&registered, "sync"), json!({}))
        .await
        .expect_err("sync requires connection id");
    assert!(missing_sync.contains("missing required param 'connection_id'"));

    let blank_available = composio_call(
        composio_controller(&registered, "list_available_triggers"),
        json!({ "toolkit": " " }),
    )
    .await
    .expect_err("available triggers rejects blank toolkit");
    assert!(blank_available.contains("'toolkit' must not be empty"));

    let missing_enable_connection = composio_call(
        composio_controller(&registered, "enable_trigger"),
        json!({ "connection_id": " ", "slug": "GMAIL_NEW_GMAIL_MESSAGE" }),
    )
    .await
    .expect_err("enable trigger rejects blank connection");
    assert!(missing_enable_connection.contains("'connection_id' must not be empty"));

    let missing_disable = composio_call(
        composio_controller(&registered, "disable_trigger"),
        json!({}),
    )
    .await
    .expect_err("disable trigger requires id");
    assert!(missing_disable.contains("missing required param 'trigger_id'"));

    let bad_set_key = composio_call(
        composio_controller(&registered, "set_api_key"),
        json!({ "api_key": "" }),
    )
    .await
    .expect_err("set api key requires non-empty key");
    assert!(bad_set_key.contains("'api_key' must not be empty"));

    let bad_github_repos = composio_call(
        composio_controller(&registered, "list_github_repos"),
        json!({ "connection_id": 42 }),
    )
    .await
    .expect_err("github repos connection id must be string");
    assert!(bad_github_repos.contains("invalid params"));

    let bad_history_limit = composio_call(
        composio_controller(&registered, "list_trigger_history"),
        json!({ "limit": "many" }),
    )
    .await
    .expect_err("history limit must be numeric");
    assert!(bad_history_limit.contains("invalid params"));

    let bad_list_triggers = composio_call(
        composio_controller(&registered, "list_triggers"),
        json!({ "toolkit": 12 }),
    )
    .await
    .expect_err("list triggers toolkit must be string");
    assert!(bad_list_triggers.contains("invalid params"));

    let missing_enable_slug = composio_call(
        composio_controller(&registered, "enable_trigger"),
        json!({ "connection_id": "conn-1", "slug": " " }),
    )
    .await
    .expect_err("enable trigger rejects blank slug");
    assert!(missing_enable_slug.contains("'slug' must not be empty"));
}

fn composio_controller<'a>(
    controllers: &'a [RegisteredController],
    function: &str,
) -> &'a RegisteredController {
    controllers
        .iter()
        .find(|controller| controller.schema.function == function)
        .unwrap_or_else(|| panic!("controller {function} registered"))
}

async fn composio_call(controller: &RegisteredController, params: Value) -> Result<Value, String> {
    let params = params.as_object().cloned().unwrap_or_default();
    (controller.handler)(params).await
}

#[tokio::test]
async fn composio_agent_tools_cover_metadata_missing_params_and_scope_helpers() {
    let dir = tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    let config = Arc::new(config);

    let list_toolkits = ComposioListToolkitsTool::new(config.clone());
    assert_eq!(list_toolkits.name(), "composio_list_toolkits");
    assert_eq!(list_toolkits.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(list_toolkits.category(), ToolCategory::Workflow);
    assert_eq!(
        list_toolkits
            .parameters_schema()
            .pointer("/additionalProperties"),
        Some(&json!(false))
    );

    let list_connections = ComposioListConnectionsTool::new(config.clone());
    assert_eq!(list_connections.name(), "composio_list_connections");
    assert_eq!(
        list_connections.permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(list_connections.category(), ToolCategory::Workflow);

    let authorize = ComposioAuthorizeTool::new(config.clone());
    assert_eq!(authorize.name(), "composio_authorize");
    assert_eq!(authorize.permission_level(), PermissionLevel::Write);
    assert_eq!(
        authorize.parameters_schema().pointer("/required/0"),
        Some(&json!("toolkit"))
    );
    let auth_missing = authorize.execute(json!({})).await.expect("missing toolkit");
    assert!(auth_missing.is_error);
    assert!(serde_json::to_string(&auth_missing)
        .unwrap()
        .contains("'toolkit' is required"));

    let list_tools = ComposioListToolsTool::new(config.clone());
    assert_eq!(list_tools.name(), "composio_list_tools");
    assert!(list_tools.supports_markdown());
    assert_eq!(
        list_tools
            .parameters_schema()
            .pointer("/properties/tags/items/type"),
        Some(&json!("string"))
    );

    let execute = ComposioExecuteTool::new(config.clone());
    assert_eq!(execute.name(), "composio_execute");
    assert_eq!(execute.permission_level(), PermissionLevel::Write);
    assert_eq!(execute.category(), ToolCategory::Workflow);
    let execute_missing = execute.execute(json!({})).await.expect("missing tool");
    assert!(execute_missing.is_error);
    assert!(serde_json::to_string(&execute_missing)
        .unwrap()
        .contains("'tool' is required"));

    let mut direct_config = (*config).clone();
    direct_config.composio.mode = "direct".to_string();
    direct_config.composio.api_key = Some("test-direct-key".to_string());
    let registered_tools = all_composio_agent_tools(&direct_config);
    let names: Vec<&str> = registered_tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(
        names,
        vec![
            "composio_list_toolkits",
            "composio_list_connections",
            "composio_authorize",
            "composio_connect",
            "composio_list_tools",
            "composio_execute",
        ]
    );
    let no_tools = all_composio_agent_tools(&Config::default());
    assert!(no_tools.is_empty());

    assert_eq!(
        toolkit_from_slug(" GMAIL_SEND_EMAIL "),
        Some("gmail".into())
    );
    assert_eq!(
        toolkit_from_slug("noUnderscore"),
        Some("nounderscore".into())
    );
    assert_eq!(toolkit_from_slug(""), None);
    assert_eq!(classify_unknown("GMAIL_DELETE_EMAIL"), ToolScope::Admin);
    assert_eq!(classify_unknown("GMAIL_SEND_EMAIL"), ToolScope::Write);
    assert_eq!(classify_unknown("GMAIL_FETCH_EMAILS"), ToolScope::Read);
    let catalog = [
        CuratedTool {
            slug: "GMAIL_FETCH_EMAILS",
            scope: ToolScope::Read,
        },
        CuratedTool {
            slug: "GMAIL_SEND_EMAIL",
            scope: ToolScope::Write,
        },
    ];
    assert_eq!(
        find_curated(&catalog, "gmail_send_email")
            .expect("case-insensitive curated match")
            .scope,
        ToolScope::Write
    );
    assert!(find_curated(&catalog, "GMAIL_DELETE_EMAIL").is_none());
    assert_eq!(ToolScope::Admin.as_str(), "admin");
    let pref = UserScopePref {
        read: true,
        write: false,
        admin: false,
    };
    assert!(pref.allows(ToolScope::Read));
    assert!(!pref.allows(ToolScope::Write));
    assert!(!pref.allows(ToolScope::Admin));

    let fallback = list_tools
        .execute_with_options(
            json!({ "toolkits": ["unknown_toolkit"], "include_unconnected": true }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("factory failure is rendered as tool result");
    assert!(fallback.is_error);
}

#[tokio::test]
async fn composio_agent_tools_direct_mode_take_local_branches_without_backend() {
    let dir = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: dir.path().join("workspace"),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    config.composio.mode = "direct".into();
    config.composio.api_key = Some("direct-test-key".into());
    config
        .save()
        .await
        .expect("persist direct-mode test config");
    let config = Arc::new(config);

    let list_toolkits = ComposioListToolkitsTool::new(config.clone());
    let toolkits = list_toolkits
        .execute(json!({}))
        .await
        .expect("direct list_toolkits returns local empty response");
    assert!(!toolkits.is_error);
    assert_eq!(toolkits.text(), r#"{"toolkits":[]}"#);

    let list_tools = ComposioListToolsTool::new(config.clone());
    let tools = list_tools
        .execute_with_options(
            json!({ "include_unconnected": true, "tags": ["ignored outside github"] }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("direct list_tools returns local empty response");
    assert!(!tools.is_error);
    assert_eq!(tools.text(), r#"{"tools":[]}"#);
    assert_eq!(
        tools.markdown_formatted.as_deref(),
        Some("_No composio tools available._")
    );

    let authorize = ComposioAuthorizeTool::new(config.clone());
    let authorize_result = authorize
        .execute(json!({ "toolkit": "gmail" }))
        .await
        .expect("direct authorize is refused locally");
    assert!(authorize_result.is_error);
    assert!(authorize_result.text().contains("direct mode is active"));

    let execute = ComposioExecuteTool::new(config);
    let execute_result = execute
        .execute(json!({
            "tool": "GMAIL_FETCH_EMAILS",
            "connection_id": "conn-gmail",
            "arguments": "not an object"
        }))
        .await
        .expect("direct execute validation is rendered locally");
    assert!(execute_result.is_error);
    let execute_text = execute_result.text();
    assert!(
        execute_text.starts_with("[composio:error:"),
        "{execute_text}"
    );
    assert!(execute_text.contains("must be a JSON object"));
}

#[test]
fn composio_types_roundtrip_connection_tool_trigger_and_history_shapes() {
    let toolkits: ComposioToolkitsResponse = serde_json::from_value(json!({})).unwrap();
    assert!(toolkits.toolkits.is_empty());

    let agent_ready = ComposioAgentReadyToolkitsResponse {
        toolkits: vec!["gmail".into(), "googlesheets".into()],
    };
    assert_eq!(
        serde_json::to_value(&agent_ready).unwrap()["toolkits"][1],
        "googlesheets"
    );

    let authorize = ComposioAuthorizeResponse {
        connect_url: "https://connect.example/oauth".into(),
        connection_id: "conn-123".into(),
    };
    let authorize_json = serde_json::to_value(&authorize).unwrap();
    assert_eq!(
        authorize_json["connectUrl"],
        "https://connect.example/oauth"
    );
    assert_eq!(authorize_json["connectionId"], "conn-123");

    let capabilities = ComposioCapabilitiesResponse {
        capabilities: vec![ComposioCapability {
            toolkit: "gmail".into(),
            description: "Gmail".into(),
            native_provider: true,
            curated_tools: true,
            curated_tool_count: 3,
            tool_execution: true,
            user_profile: true,
            initial_sync: true,
            periodic_sync: true,
            sync_interval_secs: Some(3600),
            trigger_webhooks: true,
            memory_ingest: true,
        }],
    };
    assert_eq!(
        serde_json::to_value(&capabilities).unwrap()["capabilities"][0]["toolkit"],
        "gmail"
    );

    let connections: ComposioConnectionsResponse = serde_json::from_value(json!({
        "connections": [
            { "id": "c1", "toolkit": " Gmail ", "status": " connected ", "createdAt": "2026-05-29T00:00:00Z" },
            { "id": "c2", "toolkit": "slack", "status": "PENDING" }
        ]
    }))
    .unwrap();
    assert_eq!(connections.connections[0].normalized_toolkit(), "gmail");
    assert!(connections.connections[0].is_active());
    assert!(!connections.connections[1].is_active());
    let serialized_connection = serde_json::to_value(&connections.connections[0]).unwrap();
    assert_eq!(serialized_connection["createdAt"], "2026-05-29T00:00:00Z");

    let default_connection = ComposioConnection {
        id: "c3".into(),
        toolkit: "notion".into(),
        status: "FAILED".into(),
        created_at: None,
        account_email: None,
        workspace: None,
        username: None,
    };
    assert!(serde_json::to_value(default_connection)
        .unwrap()
        .get("createdAt")
        .is_none());

    let tools = ComposioToolsResponse {
        tools: vec![ComposioToolSchema {
            kind: "function".into(),
            function: ComposioToolFunction {
                name: "GMAIL_SEND_EMAIL".into(),
                description: Some("Send mail".into()),
                parameters: Some(json!({ "type": "object" })),
            },
        }],
    };
    assert_eq!(
        serde_json::to_value(&tools).unwrap()["tools"][0]["type"],
        "function"
    );
    let default_kind: ComposioToolSchema = serde_json::from_value(json!({
        "function": { "name": "SLACK_SENDS_A_MESSAGE_TO_A_SLACK_CHANNEL" }
    }))
    .unwrap();
    assert_eq!(default_kind.kind, "function");
    assert_eq!(default_kind.function.description, None);

    let execute: ComposioExecuteResponse = serde_json::from_value(json!({
        "data": { "id": "msg-1" },
        "successful": true,
        "costUsd": 0.03,
        "markdownFormatted": "**sent**"
    }))
    .unwrap();
    assert!(execute.successful);
    assert_eq!(execute.cost_usd, 0.03);
    assert_eq!(execute.markdown_formatted.as_deref(), Some("**sent**"));

    let repos = ComposioGithubReposResponse {
        connection_id: "conn-github".into(),
        repositories: vec![ComposioGithubRepo {
            owner: "tinyhumansai".into(),
            repo: "openhuman".into(),
            full_name: "tinyhumansai/openhuman".into(),
            private: Some(false),
            default_branch: Some("main".into()),
            html_url: Some("https://github.com/tinyhumansai/openhuman".into()),
        }],
    };
    assert_eq!(
        serde_json::to_value(&repos).unwrap()["connectionId"],
        "conn-github"
    );

    let create = ComposioCreateTriggerResponse {
        trigger_id: "trig-1".into(),
        status: Some("enabled".into()),
    };
    assert_eq!(
        serde_json::to_value(&create).unwrap()["triggerId"],
        "trig-1"
    );
    let available = ComposioAvailableTriggersResponse {
        triggers: vec![ComposioAvailableTrigger {
            slug: "GITHUB_PULL_REQUEST_EVENT".into(),
            scope: "github_repo".into(),
            default_config: Some(json!({ "event": "pull_request" })),
            required_config_keys: Some(vec!["owner".into(), "repo".into()]),
            repo: Some(ComposioAvailableTriggerRepo {
                owner: "tinyhumansai".into(),
                repo: "openhuman".into(),
            }),
        }],
    };
    assert_eq!(
        serde_json::to_value(&available).unwrap()["triggers"][0]["repo"]["repo"],
        "openhuman"
    );

    let active: ComposioActiveTriggersResponse = serde_json::from_value(json!({
        "triggers": [{
            "id": { "id": "trigger-id" },
            "slug": { "slug": "GMAIL_NEW_GMAIL_MESSAGE" },
            "toolkit": { "name": "gmail" },
            "connectionId": { "key": "conn-1" },
            "triggerConfig": { "label": "INBOX" },
            "state": { "state": "enabled" }
        }]
    }))
    .unwrap();
    let active_trigger: &ComposioActiveTrigger = &active.triggers[0];
    assert_eq!(active_trigger.id, "trigger-id");
    assert_eq!(active_trigger.slug, "GMAIL_NEW_GMAIL_MESSAGE");
    assert_eq!(active_trigger.toolkit, "gmail");
    assert_eq!(active_trigger.connection_id, "conn-1");
    assert_eq!(active_trigger.state.as_deref(), Some("enabled"));
    let active_without_state: ComposioActiveTrigger = serde_json::from_value(json!({
        "id": "trigger-2",
        "slug": "SLACK_NEW_MESSAGE",
        "toolkit": "slack",
        "connectionId": "conn-2",
        "state": { "unexpected": true }
    }))
    .unwrap();
    assert_eq!(active_without_state.state, None);
    let active_string_state: ComposioActiveTrigger = serde_json::from_value(json!({
        "id": "trigger-2b",
        "slug": "SLACK_NEW_MESSAGE",
        "toolkit": "slack",
        "connectionId": "conn-2",
        "state": "READY"
    }))
    .unwrap();
    assert_eq!(active_string_state.state.as_deref(), Some("READY"));
    let active_null_state: ComposioActiveTrigger = serde_json::from_value(json!({
        "id": "trigger-2c",
        "slug": "SLACK_NEW_MESSAGE",
        "toolkit": "slack",
        "connectionId": "conn-2",
        "state": null
    }))
    .unwrap();
    assert_eq!(active_null_state.state, None);
    let active_numeric_state: ComposioActiveTrigger = serde_json::from_value(json!({
        "id": "trigger-2d",
        "slug": "SLACK_NEW_MESSAGE",
        "toolkit": "slack",
        "connectionId": "conn-2",
        "state": 12
    }))
    .unwrap();
    assert_eq!(active_numeric_state.state, None);
    assert!(serde_json::from_value::<ComposioActiveTrigger>(json!({
        "id": ["bad"],
        "slug": "x",
        "toolkit": "gmail",
        "connectionId": "c"
    }))
    .is_err());
    for bad_id in [json!(null), json!(true), json!(123)] {
        assert!(serde_json::from_value::<ComposioActiveTrigger>(json!({
            "id": bad_id,
            "slug": "x",
            "toolkit": "gmail",
            "connectionId": "c"
        }))
        .is_err());
    }
    let missing_nested_slug = serde_json::from_value::<ComposioActiveTrigger>(json!({
        "id": "trigger-3",
        "slug": { "unexpected": true },
        "toolkit": "gmail",
        "connectionId": "c"
    }))
    .expect_err("nested slug object needs a known string key");
    assert!(missing_nested_slug.to_string().contains("slug/id/name/key"));

    let enable = ComposioEnableTriggerResponse {
        trigger_id: "trig-2".into(),
        slug: "SLACK_NEW_MESSAGE".into(),
        connection_id: "conn-2".into(),
    };
    assert_eq!(
        serde_json::to_value(&enable).unwrap()["connectionId"],
        "conn-2"
    );
    assert!(
        serde_json::to_value(ComposioDisableTriggerResponse { deleted: false })
            .unwrap()
            .get("deleted")
            .is_some()
    );
    assert_eq!(
        serde_json::to_value(ComposioDeleteResponse {
            deleted: true,
            memory_chunks_deleted: 4,
        })
        .unwrap()["memory_chunks_deleted"],
        4
    );

    let event: ComposioTriggerEvent = serde_json::from_value(json!({
        "toolkit": "gmail",
        "trigger": "GMAIL_NEW_GMAIL_MESSAGE",
        "payload": { "subject": "coverage" },
        "metadata": { "id": "m1", "uuid": "u1" }
    }))
    .unwrap();
    assert_eq!(event.metadata.id, "m1");
    assert_eq!(event.payload["subject"], "coverage");
    let default_event: ComposioTriggerEvent = serde_json::from_value(json!({})).unwrap();
    assert_eq!(default_event.metadata.uuid, "");
    let metadata = ComposioTriggerMetadata {
        id: "m2".into(),
        uuid: "u2".into(),
    };
    assert_eq!(serde_json::to_value(metadata).unwrap()["uuid"], "u2");
    let entry = ComposioTriggerHistoryEntry {
        received_at_ms: 42,
        toolkit: "gmail".into(),
        trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
        metadata_id: "m1".into(),
        metadata_uuid: "u1".into(),
        payload: json!({ "subject": "coverage" }),
    };
    assert_eq!(serde_json::to_value(entry).unwrap()["received_at_ms"], 42);
    let history = ComposioTriggerHistoryResult {
        archive_dir: "/tmp/archive".into(),
        current_day_file: "/tmp/archive/2026-05-29.jsonl".into(),
        entries: vec![ComposioTriggerHistoryEntry {
            received_at_ms: 43,
            toolkit: "slack".into(),
            trigger: "SLACK_NEW_MESSAGE".into(),
            metadata_id: "m2".into(),
            metadata_uuid: "u2".into(),
            payload: json!({ "text": "coverage" }),
        }],
    };
    let history_json = serde_json::to_value(history).unwrap();
    assert_eq!(history_json["entries"][0]["metadata_uuid"], "u2");
}

#[test]
fn composio_direct_public_types_deserialize_polymorphic_toolkits() {
    let action: ComposioAction = serde_json::from_value(json!({
        "name": "GMAIL_SEND_EMAIL",
        "appName": "gmail",
        "description": "Send email"
    }))
    .unwrap();
    assert_eq!(action.name, "GMAIL_SEND_EMAIL");
    assert_eq!(action.app_name.as_deref(), Some("gmail"));
    assert!(!action.enabled);
    assert_eq!(
        serde_json::to_value(&action).unwrap()["appName"],
        json!("gmail")
    );

    let plain: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "acct-1",
        "status": "ACTIVE",
        "createdAt": "2026-05-29T00:00:00Z",
        "toolkit": " gmail "
    }))
    .unwrap();
    assert_eq!(plain.toolkit_slug().as_deref(), Some("gmail"));
    assert_eq!(plain.created_at.as_deref(), Some("2026-05-29T00:00:00Z"));

    let nested: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "acct-2",
        "toolkit": { "key": "slack" }
    }))
    .unwrap();
    assert_eq!(nested.toolkit_slug().as_deref(), Some("slack"));

    for (field, expected) in [
        ("slug", "github"),
        ("id", "googlecalendar"),
        ("name", "googledrive"),
    ] {
        let account: ComposioConnectedAccount = serde_json::from_value(json!({
            "id": format!("acct-{field}"),
            "toolkit": { field: format!(" {expected} ") }
        }))
        .unwrap();
        assert_eq!(account.toolkit_slug().as_deref(), Some(expected));
    }

    let fallback: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "acct-3",
        "toolkit": { "ignored": "value" },
        "app_name": " notion "
    }))
    .unwrap();
    assert_eq!(fallback.toolkit_slug().as_deref(), Some("notion"));

    let missing: ComposioConnectedAccount = serde_json::from_value(json!({
        "id": "acct-4",
        "toolkit": ["bad"],
        "appName": " "
    }))
    .unwrap();
    assert_eq!(missing.toolkit_slug(), None);
}

#[tokio::test]
async fn composio_backend_client_methods_build_requests_and_parse_local_envelopes() {
    let app = Router::new().fallback(any(composio_round8_backend_handler));
    let base = start_composio_round8_backend(app).await;
    let client = ComposioClient::new(Arc::new(IntegrationClient::new(
        base,
        "round8-token".into(),
    )));

    let toolkits = client.list_toolkits().await.expect("toolkits");
    assert_eq!(toolkits.toolkits, vec!["gmail", "github", "slack"]);

    let connections = client.list_connections().await.expect("connections");
    assert_eq!(connections.connections.len(), 2);
    assert_eq!(connections.connections[0].normalized_toolkit(), "gmail");

    let authorize = client
        .authorize(
            " Gmail ",
            Some(json!({
                "waba_id": "waba-1",
                "oauth_scopes": "profile https://www.googleapis.com/auth/gmail.readonly"
            })),
        )
        .await
        .expect("authorize with string scopes");
    assert_eq!(authorize.connect_url, "https://connect.example/gmail");
    assert_eq!(authorize.connection_id, "conn-gmail");

    let authorize_array = client
        .authorize("gmail", Some(json!({ "oauth_scopes": ["profile", ""] })))
        .await
        .expect("authorize appends missing array scopes");
    assert_eq!(authorize_array.connection_id, "conn-gmail");

    let bad_scope_entry = client
        .authorize("gmail", Some(json!({ "oauth_scopes": [42] })))
        .await
        .expect_err("scope entries must be strings");
    assert!(bad_scope_entry
        .to_string()
        .contains("entries must be strings"));
    let bad_scope_shape = client
        .authorize("gmail", Some(json!({ "oauth_scopes": { "bad": true } })))
        .await
        .expect_err("scope shape must be string or array");
    assert!(bad_scope_shape
        .to_string()
        .contains("must be a string or array"));

    let tools = client
        .list_tools(
            Some(&[
                " gmail ".to_string(),
                "".to_string(),
                "github/repo".to_string(),
            ]),
            Some(&[" important tag ".to_string(), " ".to_string()]),
        )
        .await
        .expect("tools");
    assert_eq!(tools.tools[0].function.name, "GMAIL_SEND_EMAIL");

    let all_tools = client.list_tools(None, None).await.expect("all tools");
    assert_eq!(all_tools.tools.len(), 1);

    let execute = client
        .execute_tool(
            " GMAIL_SEND_EMAIL ",
            Some(json!({ "to": "p@example.test" })),
        )
        .await
        .expect("execute");
    assert!(execute.successful);
    assert_eq!(execute.data["id"], "msg-1");

    let dispatched = execute_composio_action(
        &client,
        "GMAIL_SEND_EMAIL",
        Some(json!({ "to": "p@example.test", "subject": "hello" })),
    )
    .await
    .expect("dispatch uses auth-retry wrapper and local backend");
    assert!(dispatched.successful);
    assert_eq!(dispatched.data["id"], "msg-1");

    let execute_error = client
        .execute_tool(
            "GMAIL_FETCH_EMAILS",
            Some(json!({ "query": "newer_than:1d" })),
        )
        .await
        .expect("execute provider failure envelope");
    assert!(!execute_error.successful);
    assert!(execute_error
        .error
        .as_deref()
        .unwrap_or_default()
        .starts_with("[composio:error:insufficient_scope]"));

    let dispatched_error = execute_composio_action(
        &client,
        "GMAIL_FETCH_EMAILS",
        Some(json!({ "query": "newer_than:1d" })),
    )
    .await
    .expect("provider failures stay in response envelope");
    assert!(!dispatched_error.successful);
    assert!(dispatched_error
        .error
        .as_deref()
        .unwrap_or_default()
        .starts_with("[composio:error:insufficient_scope]"));

    let repos = client
        .list_github_repos(Some(" github conn "))
        .await
        .expect("repos");
    assert_eq!(repos.repositories[0].full_name, "tinyhumansai/openhuman");
    let repos_without_connection = client
        .list_github_repos(None)
        .await
        .expect("repos without connection");
    assert_eq!(repos_without_connection.connection_id, "conn-github");

    let created = client
        .create_trigger(
            " GITHUB_PULL_REQUEST_EVENT ",
            Some(" conn-github "),
            Some(json!({ "owner": "tinyhumansai", "repo": "openhuman" })),
        )
        .await
        .expect("create trigger");
    assert_eq!(created.trigger_id, "created-trigger");

    let created_without_config = client
        .create_trigger("SLACK_NEW_MESSAGE", None, None)
        .await
        .expect("create trigger without optional fields");
    assert_eq!(created_without_config.status.as_deref(), Some("enabled"));

    let available = client
        .list_available_triggers(" github ", Some(" conn-github "))
        .await
        .expect("available triggers");
    assert_eq!(available.triggers[0].slug, "GITHUB_PULL_REQUEST_EVENT");
    let available_without_connection = client
        .list_available_triggers("gmail", None)
        .await
        .expect("available triggers without connection");
    assert_eq!(available_without_connection.triggers[0].scope, "mailbox");

    let active = client
        .list_active_triggers(Some(" gmail "))
        .await
        .expect("active triggers");
    assert_eq!(active.triggers[0].toolkit, "gmail");
    let active_all = client
        .list_active_triggers(None)
        .await
        .expect("all active triggers");
    assert_eq!(active_all.triggers[0].id, "active-trigger");

    let enabled = client
        .enable_trigger(
            " conn-gmail ",
            " GMAIL_NEW_GMAIL_MESSAGE ",
            Some(json!({ "label": "INBOX" })),
        )
        .await
        .expect("enable trigger");
    assert_eq!(enabled.connection_id, "conn-gmail");
    let enabled_without_config = client
        .enable_trigger("conn-gmail", "GMAIL_NEW_GMAIL_MESSAGE", None)
        .await
        .expect("enable trigger without config");
    assert_eq!(enabled_without_config.slug, "GMAIL_NEW_GMAIL_MESSAGE");

    let deleted = client
        .delete_connection(" conn-gmail ")
        .await
        .expect("delete connection");
    assert!(deleted.deleted);
    let disabled = client
        .disable_trigger(" trigger/with space ")
        .await
        .expect("disable trigger");
    assert!(disabled.deleted);

    let delete_status = client
        .delete_connection("bad-status")
        .await
        .expect_err("delete non-2xx");
    assert!(delete_status.to_string().contains("Backend returned"));
    assert!(delete_status.to_string().contains("delete rejected"));

    let delete_envelope = client
        .delete_connection("bad-envelope")
        .await
        .expect_err("delete envelope error");
    assert!(delete_envelope
        .to_string()
        .contains("Backend error for DELETE"));

    let delete_no_data = client
        .disable_trigger("no-data")
        .await
        .expect_err("delete success needs data");
    assert!(delete_no_data.to_string().contains("success but no data"));
}

#[tokio::test]
async fn composio_authorize_scope_merging_and_meta_cleanup_use_local_backend() {
    #[derive(Clone, Default)]
    struct CleanupState {
        deleted: Arc<Mutex<Vec<String>>>,
        authorize_bodies: Arc<Mutex<Vec<Value>>>,
    }

    async fn handler(State(state): State<CleanupState>, request: Request) -> Response {
        let method = request.method().clone();
        let path = request.uri().path().to_string();
        let body = to_bytes(request.into_body(), usize::MAX)
            .await
            .expect("mock request body");
        let body: Value = if body.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&body).expect("json request body")
        };

        match (method, path.as_str()) {
            (Method::GET, "/agent-integrations/composio/connections") => ok(json!({
                "connections": [
                    { "id": "ig-active", "toolkit": "instagram", "status": "ACTIVE" },
                    { "id": "ig-failed", "toolkit": "instagram", "status": "FAILED" },
                    { "id": "ig-pending", "toolkit": " Instagram ", "status": "pending" },
                    { "id": "ig-expired", "toolkit": "instagram", "status": "EXPIRED" },
                    { "id": "fb-pending", "toolkit": "facebook", "status": "PENDING" },
                    { "id": "gmail-pending", "toolkit": "gmail", "status": "PENDING" }
                ]
            })),
            (Method::DELETE, path)
                if path.starts_with("/agent-integrations/composio/connections/") =>
            {
                state
                    .deleted
                    .lock()
                    .expect("deleted ids")
                    .push(path.rsplit('/').next().unwrap_or_default().to_string());
                ok(json!({ "deleted": true, "memory_chunks_deleted": 0 }))
            }
            (Method::POST, "/agent-integrations/composio/authorize") => {
                state
                    .authorize_bodies
                    .lock()
                    .expect("authorize bodies")
                    .push(body.clone());
                ok(json!({
                    "connectUrl": format!(
                        "https://connect.example/{}",
                        body.get("toolkit").and_then(Value::as_str).unwrap_or("unknown")
                    ),
                    "connectionId": "conn-authorize"
                }))
            }
            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({ "success": false, "error": format!("unhandled {path}") })),
            )
                .into_response(),
        }
    }

    let state = CleanupState::default();
    let app = Router::new()
        .fallback(any(handler))
        .with_state(state.clone());
    let base = start_composio_round8_backend(app).await;
    let client = ComposioClient::new(Arc::new(IntegrationClient::new(
        base,
        "round12-token".into(),
    )));

    assert_eq!(
        clear_non_active_connections(&client, "gmail")
            .await
            .expect("non-meta cleanup is a no-op"),
        0
    );
    assert_eq!(
        clear_non_active_connections(&client, " Instagram ")
            .await
            .expect("stale instagram rows are deleted"),
        3
    );
    let mut deleted = state.deleted.lock().expect("deleted ids").clone();
    deleted.sort();
    assert_eq!(deleted, vec!["ig-expired", "ig-failed", "ig-pending"]);

    client
        .authorize("gmail", Some(json!({ "oauth_scopes": null })))
        .await
        .expect("null scopes are replaced with required gmail scopes");
    client
        .authorize("gmail", None)
        .await
        .expect("missing scopes get required gmail scope");
    client
        .authorize("slack", Some(json!({ "bot_scope": "chat:write" })))
        .await
        .expect("non-gmail authorize passes through extra params");

    let bodies = state.authorize_bodies.lock().expect("authorize bodies");
    assert_eq!(
        bodies[0]["oauth_scopes"],
        json!(["https://www.googleapis.com/auth/gmail.readonly"])
    );
    assert_eq!(
        bodies[1]["oauth_scopes"],
        json!(["https://www.googleapis.com/auth/gmail.readonly"])
    );
    assert!(bodies[2].get("oauth_scopes").is_none());
    assert_eq!(bodies[2]["bot_scope"], "chat:write");
}

#[tokio::test]
async fn composio_direct_tool_public_surface_handles_local_metadata_and_errors() {
    let tool = ComposioTool::new(
        "  direct-api-key  ",
        Some("  entity-123  "),
        Arc::new(SecurityPolicy::default()),
    );

    assert_eq!(tool.name(), "composio");
    assert!(tool.description().contains("1000+ apps"));
    assert_eq!(tool.category(), ToolCategory::Workflow);
    assert!(tool.external_effect());
    assert!(!tool.external_effect_with_args(&json!({ "action": "list" })));
    assert!(!tool.external_effect_with_args(&json!({ "action": "connect" })));
    assert!(tool.external_effect_with_args(&json!({ "action": "execute" })));
    assert!(tool.external_effect_with_args(&json!({})));
    assert_eq!(
        tool.parameters_schema().pointer("/required/0"),
        Some(&json!("action"))
    );
    assert_eq!(
        tool.parameters_schema()
            .pointer("/properties/action/enum/2"),
        Some(&json!("connect"))
    );

    let missing_action = tool
        .execute(json!({}))
        .await
        .expect_err("missing action is a local validation error");
    assert!(missing_action.to_string().contains("Missing 'action'"));

    let unknown = tool
        .execute(json!({ "action": "inspect" }))
        .await
        .expect("unknown action is rendered as a tool error");
    assert!(unknown.is_error);
    assert!(serde_json::to_string(&unknown)
        .unwrap()
        .contains("Unknown action 'inspect'"));

    let missing_execute_name = tool
        .execute(json!({ "action": "execute", "params": { "q": "test" } }))
        .await
        .expect_err("execute needs action_name or tool_slug before network");
    assert!(missing_execute_name
        .to_string()
        .contains("Missing 'action_name'"));

    let missing_connect_target = tool
        .execute(json!({ "action": "connect" }))
        .await
        .expect_err("connect needs app or auth config before network");
    assert!(missing_connect_target
        .to_string()
        .contains("Missing 'app' or 'auth_config_id'"));

    let read_only_tool = ComposioTool::new(
        "direct-api-key",
        Some("default"),
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        }),
    );
    let blocked_execute = read_only_tool
        .execute(json!({
            "action": "execute",
            "tool_slug": "GMAIL_FETCH_EMAILS",
            "params": { "query": "newer_than:1d" }
        }))
        .await
        .expect("policy block is rendered as a tool result");
    assert!(blocked_execute.is_error);
    assert!(serde_json::to_string(&blocked_execute)
        .unwrap()
        .contains("read-only mode"));

    let blocked_connect = read_only_tool
        .execute(json!({
            "action": "connect",
            "auth_config_id": "auth-config-1"
        }))
        .await
        .expect("policy block is rendered as a tool result");
    assert!(blocked_connect.is_error);
    assert!(serde_json::to_string(&blocked_connect)
        .unwrap()
        .contains("read-only mode"));
}

#[test]
fn composio_trigger_history_store_handles_limits_and_bad_archive_lines() {
    let dir = tempdir().expect("tempdir");
    let store = ComposioTriggerHistoryStore::new(dir.path()).expect("history store");
    let empty = store.list_recent(0).expect("empty history");
    assert!(empty.entries.is_empty());
    assert!(empty.archive_dir.ends_with("state/triggers"));

    let first = store
        .record_trigger(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "metadata-1",
            "uuid-1",
            &json!({ "subject": "first" }),
        )
        .expect("record first");
    assert_eq!(first.toolkit, "gmail");
    let second = store
        .record_trigger(
            "slack",
            "SLACK_NEW_MESSAGE",
            "metadata-2",
            "uuid-2",
            &json!({ "text": "second" }),
        )
        .expect("record second");
    assert!(second.received_at_ms >= first.received_at_ms);

    let one = store.list_recent(1).expect("limited history");
    assert_eq!(one.entries.len(), 1);
    assert_eq!(one.entries[0].metadata_id, "metadata-2");

    std::fs::write(
        dir.path()
            .join("state")
            .join("triggers")
            .join("1999-01-01.jsonl"),
        "\nnot-json\n{\"received_at_ms\":1,\"toolkit\":\"old\",\"trigger\":\"OLD\",\"metadata_id\":\"m\",\"metadata_uuid\":\"u\",\"payload\":{}}\n",
    )
    .expect("write legacy archive");
    let all = store.list_recent(10).expect("history skips bad lines");
    assert!(all.entries.iter().any(|entry| entry.toolkit == "old"));
    assert!(all
        .entries
        .iter()
        .any(|entry| entry.metadata_id == "metadata-1"));
}

async fn start_composio_round8_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock composio backend");
    let addr = listener.local_addr().expect("mock backend addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn store_app_session_token(config: &Config, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            token,
            std::collections::HashMap::new(),
            true,
        )
        .expect("store app session token");
}

async fn composio_round8_backend_handler(request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or_default().to_string();
    let body = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("mock request body");
    let body: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body).expect("json request body")
    };

    match (method, path.as_str()) {
        (Method::GET, "/agent-integrations/composio/toolkits") => ok(json!({
            "toolkits": ["gmail", "github", "slack"]
        })),
        (Method::GET, "/agent-integrations/composio/connections") => ok(json!({
            "connections": [
                { "id": "conn-gmail", "toolkit": " Gmail ", "status": "ACTIVE", "createdAt": "2026-05-29T00:00:00Z" },
                { "id": "conn-slack", "toolkit": "slack", "status": "PENDING" }
            ]
        })),
        (Method::POST, "/agent-integrations/composio/authorize") => {
            let toolkit = body
                .get("toolkit")
                .and_then(Value::as_str)
                .expect("authorize toolkit");
            assert_eq!(toolkit.to_ascii_lowercase(), "gmail");
            let scopes = body
                .get("oauth_scopes")
                .and_then(Value::as_array)
                .expect("oauth scopes array");
            assert!(scopes
                .iter()
                .any(|scope| scope == "https://www.googleapis.com/auth/gmail.readonly"));
            ok(json!({
                "connectUrl": "https://connect.example/gmail",
                "connectionId": "conn-gmail"
            }))
        }
        (Method::GET, "/agent-integrations/composio/tools") => {
            if !query.is_empty() {
                assert!(query.contains("toolkits=gmail,github%2Frepo"));
                assert!(query.contains("tags=important%20tag"));
            }
            ok(json!({
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "GMAIL_SEND_EMAIL",
                        "description": "Send mail",
                        "parameters": { "type": "object" }
                    }
                }]
            }))
        }
        (Method::POST, "/agent-integrations/composio/execute") => {
            match body.get("tool").and_then(Value::as_str) {
                Some("GMAIL_FETCH_EMAILS") => ok(json!({
                    "data": null,
                    "successful": false,
                    "error": "403 insufficient authentication scopes",
                    "costUsd": 0.0
                })),
                _ => ok(json!({
                    "data": { "id": "msg-1" },
                    "successful": true,
                    "error": null,
                    "costUsd": 0.01,
                    "markdownFormatted": "**sent**"
                })),
            }
        }
        (Method::GET, "/agent-integrations/composio/github/repos") => ok(json!({
            "connectionId": "conn-github",
            "repositories": [{
                "owner": "tinyhumansai",
                "repo": "openhuman",
                "fullName": "tinyhumansai/openhuman",
                "private": false,
                "defaultBranch": "main",
                "htmlUrl": "https://github.com/tinyhumansai/openhuman"
            }]
        })),
        (Method::POST, "/agent-integrations/composio/triggers") => {
            if body.get("slug").and_then(Value::as_str) == Some("GITHUB_PULL_REQUEST_EVENT") {
                assert_eq!(body["connectionId"], "conn-github");
                ok(json!({
                    "triggerId": "created-trigger",
                    "status": "enabled"
                }))
            } else if body.get("connectionId").is_some() {
                assert_eq!(body["connectionId"], "conn-gmail");
                ok(json!({
                    "triggerId": "enabled-trigger",
                    "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                    "connectionId": "conn-gmail"
                }))
            } else {
                ok(json!({
                    "triggerId": "created-trigger",
                    "status": "enabled"
                }))
            }
        }
        (Method::GET, "/agent-integrations/composio/triggers/available") => {
            if query.contains("github") {
                assert!(query.contains("connectionId=conn-github"));
                ok(json!({
                    "triggers": [{
                        "slug": "GITHUB_PULL_REQUEST_EVENT",
                        "scope": "github_repo",
                        "defaultConfig": { "event": "pull_request" },
                        "requiredConfigKeys": ["owner", "repo"],
                        "repo": { "owner": "tinyhumansai", "repo": "openhuman" }
                    }]
                }))
            } else {
                ok(json!({
                    "triggers": [{
                        "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                        "scope": "mailbox"
                    }]
                }))
            }
        }
        (Method::GET, "/agent-integrations/composio/triggers") => ok(json!({
            "triggers": [{
                "id": "active-trigger",
                "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                "toolkit": "gmail",
                "connectionId": "conn-gmail",
                "triggerConfig": { "label": "INBOX" },
                "state": "enabled"
            }]
        })),
        (Method::DELETE, path) if path.starts_with("/agent-integrations/composio/connections/") => {
            let id = path.rsplit('/').next().unwrap_or_default();
            match id {
                "bad-status" => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "success": false, "error": "delete rejected" })),
                )
                    .into_response(),
                "bad-envelope" => Json(json!({
                    "success": false,
                    "error": "delete envelope rejected"
                }))
                .into_response(),
                _ => ok(json!({ "deleted": true, "memory_chunks_deleted": 2 })),
            }
        }
        (Method::DELETE, path) if path.starts_with("/agent-integrations/composio/triggers/") => {
            let id = path.rsplit('/').next().unwrap_or_default();
            if id == "no-data" {
                Json(json!({ "success": true })).into_response()
            } else {
                ok(json!({ "deleted": true }))
            }
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "error": format!("unhandled {path}") })),
        )
            .into_response(),
    }
}

fn ok(data: Value) -> Response {
    Json(json!({ "success": true, "data": data })).into_response()
}
