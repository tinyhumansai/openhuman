use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

use crate::openhuman::config::Config;
use crate::rpc::StructuredRpcError;

use super::{
    all_internal_controllers, registry_get_agent_version, registry_get_connector_binding_version,
    registry_get_connector_type_version, registry_get_tool_definition_version,
    registry_get_tool_enablement_version, registry_list_agents, registry_list_connector_bindings,
    registry_list_connector_types, registry_list_tool_definitions, registry_list_tool_enablements,
    registry_schemas, AgentRegistryAgent, AgentRegistryAgentSummary, ConnectorRegistryType,
    RegistryCursorListResponse, RegistryGetAgentVersionRpcParams,
    RegistryGetConnectorBindingVersionRpcParams, RegistryGetConnectorTypeVersionRpcParams,
    RegistryGetToolDefinitionVersionRpcParams, RegistryGetToolEnablementVersionRpcParams,
    RegistryListAgentsRpcParams, RegistryListConnectorBindingsRpcParams,
    RegistryListConnectorTypesRpcParams, RegistryListToolDefinitionsRpcParams,
    ToolRegistryToolDefinition, ToolRegistryToolDefinitionSummary,
};

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: Method,
    path_and_query: String,
    authorization: Option<String>,
    actor: Option<String>,
}

type Requests = Arc<Mutex<Vec<CapturedRequest>>>;

fn test_config(tmp: &TempDir, base: String) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        youpet: crate::openhuman::config::YouPetConfig {
            core_api_url: base,
            service_token: Some("svc-token".into()),
            workbench_actor_id: "registry-reader".into(),
            operator_user_id: Some("22222222-2222-4222-8222-222222222222".into()),
            tenant_id: Some("20000000-0000-0000-0000-000000000001".into()),
        },
        ..Config::default()
    }
}

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    format!("http://127.0.0.1:{}", addr.port())
}

async fn capture(
    State(requests): State<Requests>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    _body: Bytes,
) -> impl IntoResponse {
    requests.lock().unwrap().push(CapturedRequest {
        method,
        path_and_query: uri
            .path_and_query()
            .map(|value| value.as_str().to_string())
            .unwrap_or_else(|| uri.path().to_string()),
        authorization: headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        actor: headers
            .get("x-actor-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
    });
    axum::Json(json!({ "ok": true }))
}

fn kind_cursor(kind: &str) -> String {
    use base64::Engine as _;

    let payload = match kind {
        "agent" => json!({
            "agent_id": "20000000-0000-4000-8000-000000000101",
            "agent_key": "logical-key",
            "tenant_id": "10000000-0000-4000-8000-000000000001",
            "v": 1
        }),
        "tool_definition" => json!({
            "definition_id": "30000000-0000-4000-8000-000000000101",
            "tool_key": "logical-key",
            "v": 1
        }),
        "connector_type" => json!({
            "key": "logical-key",
            "kind": "connector_types",
            "schema_version": 1,
            "version": 7
        }),
        "connector_binding" => json!({
            "key": "logical-key",
            "kind": "connector_bindings",
            "schema_version": 1,
            "version": 7
        }),
        _ => panic!("unsupported cursor fixture kind: {kind}"),
    };
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
}

#[test]
fn registry_contract_exports_exactly_ten_internal_controllers() {
    let controllers = all_internal_controllers();
    let methods = controllers
        .iter()
        .map(|controller| controller.rpc_method_name())
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 10);
    assert_eq!(
        methods,
        vec![
            "openhuman.youpet_registry_list_agents",
            "openhuman.youpet_registry_get_agent_version",
            "openhuman.youpet_registry_list_tool_definitions",
            "openhuman.youpet_registry_get_tool_definition_version",
            "openhuman.youpet_registry_list_tool_enablements",
            "openhuman.youpet_registry_get_tool_enablement_version",
            "openhuman.youpet_registry_list_connector_types",
            "openhuman.youpet_registry_get_connector_type_version",
            "openhuman.youpet_registry_list_connector_bindings",
            "openhuman.youpet_registry_get_connector_binding_version",
        ]
    );
}

#[test]
fn registry_schemas_do_not_expose_authority_inputs() {
    let list_schema = registry_schemas("registry_list_agents");
    let input_names = list_schema
        .inputs
        .iter()
        .map(|field| field.name)
        .collect::<Vec<_>>();
    assert_eq!(input_names, vec!["limit", "cursor"]);
    for forbidden in [
        "tenantId", "coreUrl", "token", "actorId", "method", "path", "headers", "query",
    ] {
        assert!(
            !input_names.contains(&forbidden),
            "registry list schema must not expose {forbidden}"
        );
    }

    let exact_schema = registry_schemas("registry_get_connector_binding_version");
    let exact_names = exact_schema
        .inputs
        .iter()
        .map(|field| field.name)
        .collect::<Vec<_>>();
    assert_eq!(exact_names, vec!["bindingKey", "version"]);
}

#[test]
fn registry_params_reject_invalid_versions_and_cross_family_cursors() {
    use base64::Engine as _;

    for kind in [
        "agent",
        "tool_definition",
        "connector_type",
        "connector_binding",
    ] {
        let cursor = kind_cursor(kind);
        let accepted = match kind {
            "agent" => RegistryListAgentsRpcParams {
                limit: Some(50),
                cursor: Some(cursor),
            }
            .validate(),
            "tool_definition" => RegistryListToolDefinitionsRpcParams {
                limit: Some(50),
                cursor: Some(cursor),
            }
            .validate(),
            "connector_type" => RegistryListConnectorTypesRpcParams {
                limit: Some(50),
                cursor: Some(cursor),
            }
            .validate(),
            "connector_binding" => RegistryListConnectorBindingsRpcParams {
                limit: Some(50),
                cursor: Some(cursor),
            }
            .validate(),
            _ => unreachable!(),
        };
        assert!(accepted.is_ok(), "published {kind} cursor must be accepted");
    }

    let malformed_agent_cursor = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        json!({
            "agent_id": "not-a-uuid",
            "agent_key": "logical-key",
            "tenant_id": "10000000-0000-4000-8000-000000000001",
            "v": 1
        })
        .to_string(),
    );
    assert!(
        RegistryListAgentsRpcParams {
            limit: Some(50),
            cursor: Some(malformed_agent_cursor),
        }
        .validate()
        .is_err(),
        "Agent cursors must reject UUID-shaped fields that cannot be parsed"
    );

    let wrong_cursor = kind_cursor("connector_binding");
    let err = RegistryListAgentsRpcParams {
        limit: Some(50),
        cursor: Some(wrong_cursor),
    }
    .validate()
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(structured.message, "invalid Registry request");
    let data = structured.data.unwrap();
    assert_eq!(data["kind"], json!("YouPetRequestInvalid"));
    assert!(!data.to_string().contains("logical-key"));
    assert!(!data.to_string().contains("connector_binding"));

    let err = RegistryGetToolEnablementVersionRpcParams {
        tool_key: "tool.alpha".into(),
        version: 0,
    }
    .validate()
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    assert_eq!(structured.message, "invalid Registry request");
    assert_eq!(
        structured.data.unwrap()["kind"],
        json!("YouPetRequestInvalid")
    );
}

#[tokio::test]
async fn registry_request_builders_use_exact_get_paths_and_headers() {
    let requests: Requests = Default::default();
    let agent_cursor = kind_cursor("agent");
    let tool_definition_cursor = kind_cursor("tool_definition");
    let connector_type_cursor = kind_cursor("connector_type");
    let connector_binding_cursor = kind_cursor("connector_binding");
    let app = Router::new()
        .route("/api/v1/kernel/agents", get(capture))
        .route("/api/v1/kernel/agents/agent.alpha/versions/7", get(capture))
        .route("/api/v1/kernel/tool-definitions", get(capture))
        .route(
            "/api/v1/kernel/tool-definitions/tool.alpha/versions/3",
            get(capture),
        )
        .route("/api/v1/kernel/tool-enablement", get(capture))
        .route(
            "/api/v1/kernel/tool-enablement/tool.alpha/versions/5",
            get(capture),
        )
        .route("/api/v1/kernel/connector-types", get(capture))
        .route(
            "/api/v1/kernel/connector-types/wecom/versions/2",
            get(capture),
        )
        .route("/api/v1/kernel/connector-bindings", get(capture))
        .route(
            "/api/v1/kernel/connector-bindings/wecom-primary/versions/11",
            get(capture),
        )
        .with_state(requests.clone());
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let _ = registry_list_agents(
        &config,
        RegistryListAgentsRpcParams {
            limit: Some(50),
            cursor: Some(agent_cursor.clone()),
        },
    )
    .await;
    let _ = registry_get_agent_version(
        &config,
        RegistryGetAgentVersionRpcParams {
            agent_key: "agent.alpha".into(),
            version: 7,
        },
    )
    .await;
    let _ = registry_list_tool_definitions(
        &config,
        RegistryListToolDefinitionsRpcParams {
            limit: Some(50),
            cursor: Some(tool_definition_cursor.clone()),
        },
    )
    .await;
    let _ = registry_get_tool_definition_version(
        &config,
        RegistryGetToolDefinitionVersionRpcParams {
            tool_key: "tool.alpha".into(),
            version: 3,
        },
    )
    .await;
    let _ = registry_list_tool_enablements(&config).await;
    let _ = registry_get_tool_enablement_version(
        &config,
        RegistryGetToolEnablementVersionRpcParams {
            tool_key: "tool.alpha".into(),
            version: 5,
        },
    )
    .await;
    let _ = registry_list_connector_types(
        &config,
        RegistryListConnectorTypesRpcParams {
            limit: Some(50),
            cursor: Some(connector_type_cursor.clone()),
        },
    )
    .await;
    let _ = registry_get_connector_type_version(
        &config,
        RegistryGetConnectorTypeVersionRpcParams {
            connector_key: "wecom".into(),
            version: 2,
        },
    )
    .await;
    let _ = registry_list_connector_bindings(
        &config,
        RegistryListConnectorBindingsRpcParams {
            limit: Some(50),
            cursor: Some(connector_binding_cursor.clone()),
        },
    )
    .await;
    let _ = registry_get_connector_binding_version(
        &config,
        RegistryGetConnectorBindingVersionRpcParams {
            binding_key: "wecom-primary".into(),
            version: 11,
        },
    )
    .await;

    let requests = requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.path_and_query.as_str())
            .collect::<Vec<_>>(),
        vec![
            format!("/api/v1/kernel/agents?limit=50&cursor={agent_cursor}"),
            "/api/v1/kernel/agents/agent.alpha/versions/7".to_string(),
            format!("/api/v1/kernel/tool-definitions?limit=50&cursor={tool_definition_cursor}"),
            "/api/v1/kernel/tool-definitions/tool.alpha/versions/3".to_string(),
            "/api/v1/kernel/tool-enablement".to_string(),
            "/api/v1/kernel/tool-enablement/tool.alpha/versions/5".to_string(),
            format!("/api/v1/kernel/connector-types?limit=50&cursor={connector_type_cursor}"),
            "/api/v1/kernel/connector-types/wecom/versions/2".to_string(),
            format!("/api/v1/kernel/connector-bindings?limit=50&cursor={connector_binding_cursor}"),
            "/api/v1/kernel/connector-bindings/wecom-primary/versions/11".to_string(),
        ]
    );
    assert!(requests.iter().all(|request| request.method == Method::GET));
    assert!(requests
        .iter()
        .all(|request| request.authorization.as_deref() == Some("Bearer svc-token")));
    assert!(requests
        .iter()
        .all(|request| request.actor.as_deref() == Some("registry-reader")));
}

#[tokio::test]
async fn registry_http_errors_preserve_retry_after_without_leaking_body() {
    let app = Router::new().route(
        "/api/v1/kernel/agents",
        get(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::RETRY_AFTER, "2")],
                axum::Json(json!({
                    "detail": {
                        "code": "rate_limited",
                        "message": "secret provider ref",
                        "cursor": "agent-secret-cursor"
                    }
                })),
            )
        }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp, base);

    let err = registry_list_agents(
        &config,
        RegistryListAgentsRpcParams {
            limit: Some(50),
            cursor: None,
        },
    )
    .await
    .unwrap_err();
    let structured = StructuredRpcError::decode(&err).expect("structured error");
    let data = structured.data.unwrap();
    assert_eq!(
        structured.message,
        "YouPet Core request failed with HTTP 429"
    );
    assert_eq!(data["kind"], json!("YouPetCoreHttpError"));
    assert_eq!(data["youpet"]["http_status"], json!(429));
    assert_eq!(data["youpet"]["code"], json!("rate_limited"));
    assert_eq!(data["youpet"]["retry_after_seconds"], json!(2));
    let rendered = data.to_string();
    assert!(!rendered.contains("secret provider ref"));
    assert!(!rendered.contains("agent-secret-cursor"));
    assert!(!rendered.contains("svc-token"));
}

#[test]
fn registry_decoding_requires_next_cursor_for_cursor_lists() {
    let parsed: RegistryCursorListResponse<Value> = serde_json::from_value(json!({
        "items": [],
        "next_cursor": null,
        "future_field": true
    }))
    .unwrap();
    assert!(parsed.next_cursor.is_none());

    let err = serde_json::from_value::<RegistryCursorListResponse<Value>>(json!({
        "items": []
    }))
    .unwrap_err();
    assert!(err.to_string().contains("next_cursor"));
}

#[test]
fn registry_agent_detail_accepts_published_knowledge_scope_shape() {
    let agent: AgentRegistryAgent = serde_json::from_value(json!({
        "id": "agent-version-123",
        "agent_key": "agent.alpha",
        "version": 7,
        "lifecycle_state": "active",
        "configuration": {
            "schema_version": 1,
            "domain_key": "care-plan",
            "owner": {
                "actor_type": "service",
                "actor_id": "openhuman"
            },
            "allowed_tool_refs": [
                {
                    "tool_key": "tool.alpha",
                    "version": 3
                }
            ],
            "knowledge_scope_refs": [
                {
                    "source_key": "care-notes",
                    "trust_version": "2026-08-31",
                    "access_scope": "tenant"
                }
            ],
            "risk_policy_ref": {
                "policy_id": "policy.alpha",
                "policy_version": "2026-08-31"
            }
        },
        "configuration_fingerprint": "cfg_fp_123",
        "owner_actor_type": "service",
        "owner_actor_id": "openhuman",
        "created_at": "2026-08-31T12:34:56Z"
    }))
    .expect("published agent detail payload should decode");

    assert_eq!(agent.agent_key, "agent.alpha");
    assert_eq!(agent.configuration.knowledge_scope_refs.len(), 1);
    assert_eq!(
        serde_json::to_value(&agent.configuration.knowledge_scope_refs[0]).unwrap(),
        json!({
            "source_key": "care-notes",
            "trust_version": "2026-08-31",
            "access_scope": "tenant"
        })
    );
}

#[test]
fn registry_agent_detail_rejects_unknown_owner_actor_type() {
    let err = serde_json::from_value::<AgentRegistryAgent>(json!({
        "id": "agent-version-123",
        "agent_key": "agent.alpha",
        "version": 7,
        "lifecycle_state": "active",
        "configuration": {
            "schema_version": 1,
            "domain_key": "care-plan",
            "owner": {
                "actor_type": "team",
                "actor_id": "ops"
            },
            "allowed_tool_refs": [],
            "knowledge_scope_refs": [],
            "risk_policy_ref": null
        },
        "configuration_fingerprint": "cfg_fp_123",
        "owner_actor_type": "service",
        "owner_actor_id": "openhuman",
        "created_at": "2026-08-31T12:34:56Z"
    }))
    .unwrap_err();

    assert!(err.to_string().contains("actor_type"));
}

#[test]
fn registry_agent_list_summary_rejects_non_active_lifecycle() {
    for lifecycle_state in ["draft", "retired"] {
        let err = serde_json::from_value::<RegistryCursorListResponse<AgentRegistryAgentSummary>>(
            json!({
                "items": [
                    {
                        "id": "agent-version-123",
                        "agent_key": "agent.alpha",
                        "version": 7,
                        "lifecycle_state": lifecycle_state,
                        "configuration_fingerprint": "cfg_fp_123",
                        "owner_actor_type": "service",
                        "owner_actor_id": "openhuman",
                        "created_at": "2026-08-31T12:34:56Z"
                    }
                ],
                "next_cursor": null
            }),
        )
        .unwrap_err();

        assert!(err.to_string().contains("active"));
    }
}

#[test]
fn registry_tool_definition_list_summary_rejects_non_active_lifecycle() {
    for lifecycle_state in ["draft", "retired"] {
        let err = serde_json::from_value::<
            RegistryCursorListResponse<ToolRegistryToolDefinitionSummary>,
        >(json!({
            "items": [
                {
                    "tool_key": "tool.alpha",
                    "version": 3,
                    "lifecycle_state": lifecycle_state,
                    "definition_fingerprint": "def_fp_123",
                    "schema_version": 1,
                    "display_name": "Tool Alpha",
                    "description": "Reads records",
                    "tool_effect_class": "read_only",
                    "abstract_auth_scopes": ["records:read"],
                    "created_at": "2026-08-31T12:34:56Z"
                }
            ],
            "next_cursor": null
        }))
        .unwrap_err();

        assert!(err.to_string().contains("active"));
    }
}

#[test]
fn registry_tool_definition_requires_object_contract_fields() {
    let valid_payload = json!({
        "tool_key": "tool.alpha",
        "version": 3,
        "lifecycle_state": "active",
        "definition_fingerprint": "def_fp_123",
        "schema_version": 1,
        "display_name": "Tool Alpha",
        "description": "Reads records",
        "tool_effect_class": "read_only",
        "abstract_auth_scopes": ["records:read"],
        "input_schema": { "type": "object", "future_field": { "nested": true } },
        "output_schema": { "type": "object", "future_field": false },
        "timeout_defaults": { "soft_ms": 5000, "extra": "ok" },
        "retry_contract": { "policy": "none", "future_field": 1 },
        "audit_contract": { "mode": "metadata_only", "future_field": ["kept"] },
        "created_at": "2026-08-31T12:34:56Z"
    });

    let tool_definition: ToolRegistryToolDefinition = serde_json::from_value(valid_payload.clone())
        .expect("object contract fields should decode");
    assert!(tool_definition.input_schema.is_object());
    assert!(tool_definition.output_schema.is_object());
    assert!(tool_definition.timeout_defaults.is_object());
    assert!(tool_definition.retry_contract.is_object());
    assert!(tool_definition.audit_contract.is_object());

    for (field_name, invalid_value) in [
        ("input_schema", json!(["not", "object"])),
        ("output_schema", json!("not-object")),
        ("timeout_defaults", json!(42)),
        ("retry_contract", json!(true)),
        ("audit_contract", json!(null)),
    ] {
        let mut payload = valid_payload.clone();
        payload[field_name] = invalid_value;
        let err = serde_json::from_value::<ToolRegistryToolDefinition>(payload).unwrap_err();
        assert!(err.to_string().contains(field_name));
    }
}

#[test]
fn registry_connector_type_requires_object_delivery_behavior() {
    let valid_payload = json!({
        "connector_key": "wecom",
        "version": 2,
        "lifecycle_state": "active",
        "source_type": "wecom",
        "connector_type_fingerprint": "conn_fp_123",
        "capabilities": ["messages"],
        "normalization_contracts": [
            {
                "evidence_family": "chat_message",
                "kernel_event_type": "wecom.message",
                "kernel_event_schema_version": 1
            }
        ],
        "delivery_behavior": {
            "mode": "push",
            "future_field": {
                "retry": true
            }
        },
        "created_at": "2026-08-31T12:34:56Z"
    });

    let connector_type: ConnectorRegistryType = serde_json::from_value(valid_payload.clone())
        .expect("object delivery_behavior should decode");
    assert!(connector_type.delivery_behavior.is_object());

    for invalid_value in [
        json!(["push"]),
        json!("push"),
        json!(1),
        json!(false),
        json!(null),
    ] {
        let mut payload = valid_payload.clone();
        payload["delivery_behavior"] = invalid_value;
        let err = serde_json::from_value::<ConnectorRegistryType>(payload).unwrap_err();
        assert!(err.to_string().contains("delivery_behavior"));
    }
}
