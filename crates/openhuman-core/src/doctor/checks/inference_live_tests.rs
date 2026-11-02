//! Behaviour tests for the opt-in live inference checks.

use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::schema::ephemeral_route::{apply as apply_ephemeral_route, EphemeralRoute};
use crate::config::Config;
use crate::core::runtime::{CoreContext, DomainSet};
use crate::doctor::{DoctorCtx, DoctorOptions, DoctorTier, Finding, FindingSeverity};
use serde_json::{json, Value};
use tinyinference_llm::providers::overrides::{ChatModelEntry, ChatModelRequest};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CHECK_CHAT: &str = "inference/live/chat";
const TEST_BEARER: &str = "dummy-doctor-bearer";
const RESPONSE_SECRET: &str = "provider-response-must-not-leak";
const ENDPOINT_SECRET: &str = "endpoint-query-secret";

fn completion_path() -> String {
    format!("/openai/{ENDPOINT_SECRET}/chat/completions")
}

struct Fixture {
    config: Config,
    _temp: tempfile::TempDir,
}

fn isolated_config() -> Fixture {
    let temp = tempfile::tempdir().expect("temporary doctor fixture");
    let root = temp.path();
    let config = Config {
        workspace_dir: root.join("workspace"),
        action_dir: root.join("action"),
        config_path: root.join("config.toml"),
        ..Config::default()
    };
    Fixture {
        config,
        _temp: temp,
    }
}

fn config_for(server: &MockServer, model: &str) -> Fixture {
    let mut fixture = isolated_config();
    let mut config = Config {
        workspace_dir: fixture.config.workspace_dir.clone(),
        action_dir: fixture.config.action_dir.clone(),
        config_path: fixture.config.config_path.clone(),
        default_model: Some(model.to_string()),
        ..Config::default()
    };
    apply_ephemeral_route(
        &mut config,
        EphemeralRoute {
            endpoint: format!("{}/openai/{ENDPOINT_SECRET}", server.uri()),
            api_key: TEST_BEARER.to_string(),
            headers: Vec::new(),
            ..EphemeralRoute::default()
        },
    );
    fixture.config = config;
    fixture
}

fn add_cloud_provider(config: &mut Config, slug: &str, endpoint: &str, model: &str) {
    config.cloud_providers.push(CloudProviderCreds {
        id: format!("provider-id-{slug}"),
        slug: slug.to_string(),
        label: "Doctor test provider".to_string(),
        endpoint: endpoint.to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some(model.to_string()),
    });
}

fn cloud_route_fixture(
    server: &MockServer,
    providers: &[(&str, &str, &str, &str)],
    chat_slug: &str,
    reasoning_slug: &str,
) -> Fixture {
    let mut fixture = isolated_config();
    fixture.config.default_model = Some("provider/shared-model".into());
    let mut active_profiles = serde_json::Map::new();
    let mut profile_records = serde_json::Map::new();
    for (slug, endpoint_suffix, model, credential) in providers {
        add_cloud_provider(
            &mut fixture.config,
            slug,
            &format!("{}/{endpoint_suffix}/{ENDPOINT_SECRET}", server.uri()),
            model,
        );
        let provider = format!("provider:{slug}");
        let profile_id = format!("{provider}:default");
        active_profiles.insert(provider.clone(), Value::String(profile_id.clone()));
        profile_records.insert(
            profile_id,
            json!({
                "provider": provider,
                "profile_name": "default",
                "kind": "token",
                "token": credential,
                "metadata": {}
            }),
        );
    }
    let profile_path = fixture
        .config
        .config_path
        .parent()
        .expect("config fixture has a parent")
        .join("auth-profiles.json");
    std::fs::write(
        profile_path,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "active_profiles": active_profiles,
            "profiles": profile_records
        }))
        .expect("serialize dummy profile data"),
    )
    .expect("write dummy keys only inside this temp fixture");
    fixture.config.chat_provider = Some(format!("{chat_slug}:provider/shared-model"));
    fixture.config.reasoning_provider = Some(format!("{reasoning_slug}:provider/shared-model"));
    fixture
}

fn provider_completion_path(endpoint_suffix: &str) -> String {
    format!("/{endpoint_suffix}/{ENDPOINT_SECRET}/chat/completions")
}

fn completion_response(model: &str, content: &str) -> Value {
    json!({
        "id": "doctor-completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3}
    })
}

async fn run_live(fixture: Fixture, only: Vec<&str>) -> crate::doctor::DoctorReport {
    let expected_ids = only.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    let Fixture { config, _temp } = fixture;
    let context = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    let report = CoreContext::scope(context, async move {
        let doctor_ctx = DoctorCtx::from_config(config);
        crate::doctor::runner::run(
            &doctor_ctx,
            DoctorOptions {
                tier: DoctorTier::Live,
                only: only.into_iter().map(str::to_string).collect(),
                ..DoctorOptions::default()
            },
        )
        .await
    })
    .await
    .expect("doctor report should be returned");
    for id in expected_ids {
        assert!(
            report.checks_run.contains(&id) || report.checks_skipped.contains(&id),
            "requested id {id} was not recognized: {:?}",
            report.findings
        );
    }
    assert!(
        !report.findings.iter().any(|finding| {
            finding.check_id == "doctor/selection" && finding.severity == FindingSeverity::Error
        }),
        "an unknown check id must never masquerade as a successful selected check"
    );
    drop(_temp);
    report
}

fn finding<'a>(report: &'a crate::doctor::DoctorReport, id: &str) -> &'a Finding {
    report
        .findings
        .iter()
        .find(|finding| finding.check_id == id)
        .unwrap_or_else(|| panic!("missing finding for {id}: {:?}", report.findings))
}

#[tokio::test]
async fn live_chat_sends_the_fixed_short_prompt_and_reports_only_safe_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(completion_path()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completion_response(
                "provider/answered-model",
                RESPONSE_SECRET,
            )),
        )
        .mount(&server)
        .await;

    let report = run_live(
        config_for(&server, "provider/configured-model"),
        vec![CHECK_CHAT],
    )
    .await;
    let result = finding(&report, CHECK_CHAT);
    assert_eq!(result.severity, FindingSeverity::Pass);
    let serialized = serde_json::to_value(&report).expect("report serializes");
    assert!(
        serialized["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["check_id"] == CHECK_CHAT)
            .and_then(|item| item.get("duration_ms"))
            .is_some(),
        "live completion latency metadata is present"
    );
    assert!(
        result.message.contains("provider/answered-model")
            || result.message.contains("provider/configured-model"),
        "the finding includes only an available safe model identifier"
    );

    let requests = server
        .received_requests()
        .await
        .expect("mock records calls");
    assert_eq!(requests.len(), 1, "one completion is enough for this role");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("request is JSON");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["content"], "Reply OK");
    assert!(body["max_tokens"]
        .as_u64()
        .is_some_and(|value| (1..=5).contains(&value)));
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .map(|value| value.to_str().unwrap()),
        Some("Bearer dummy-doctor-bearer")
    );

    let serialized = serde_json::to_string(&report).expect("report serializes");
    for secret in [TEST_BEARER, ENDPOINT_SECRET, RESPONSE_SECRET] {
        assert!(!serialized.contains(secret), "report leaked {secret}");
    }
}

#[tokio::test]
async fn live_probe_uses_selected_doctor_config_inside_another_ambient_context() {
    let ambient_server = MockServer::start().await;
    let selected_server = MockServer::start().await;
    for server in [&ambient_server, &selected_server] {
        Mock::given(method("POST"))
            .and(path(completion_path()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(completion_response("provider/selected-model", "OK")),
            )
            .mount(server)
            .await;
    }
    let ambient = config_for(&ambient_server, "provider/ambient-model");
    let selected = config_for(&selected_server, "provider/selected-model");
    let context = CoreContext::for_test_with_config(DomainSet::full(), ambient.config);
    let report = CoreContext::scope(context, async move {
        crate::doctor::runner::run(
            &DoctorCtx::from_config(selected.config),
            DoctorOptions {
                tier: DoctorTier::Live,
                only: vec![CHECK_CHAT.into()],
                ..DoctorOptions::default()
            },
        )
        .await
    })
    .await
    .expect("selected config live probe returns a report");
    assert_eq!(finding(&report, CHECK_CHAT).severity, FindingSeverity::Pass);
    assert_eq!(
        ambient_server
            .received_requests()
            .await
            .expect("ambient mock records calls")
            .len(),
        0,
        "the ambient profile must not receive a selected-config probe"
    );
    assert_eq!(
        selected_server
            .received_requests()
            .await
            .expect("selected mock records calls")
            .len(),
        1
    );
}

#[tokio::test]
async fn managed_chat_probe_uses_the_inspection_only_factory_callback() {
    use crate::inference::registry::{
        register_chat_model_in, register_inspection_chat_model_in, InspectionChatModelFactory,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

    struct CountingModel(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl ChatModel<()> for CountingModel {
        async fn invoke(
            &self,
            _state: &(),
            request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            self.0.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.messages.len(), 1);
            assert_eq!(request.messages[0].text(), "Reply OK");
            assert!(request
                .max_tokens
                .is_some_and(|value| (1..=5).contains(&value)));
            Ok(ModelResponse::assistant("OK"))
        }
    }

    let extensions = crate::core::slots::Extensions::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let model_calls = Arc::clone(&calls);
    let factory: InspectionChatModelFactory = Arc::new(
        move |request: &ChatModelRequest<'_, crate::inference::registry::ProviderContext>| {
            let model_calls = Arc::clone(&model_calls);
            Box::pin(async move {
                assert_eq!(request.role, "chat");
                Ok(crate::inference::registry::InspectionChatModelBuild {
                    model: Arc::new(CountingModel(Arc::clone(&model_calls)))
                        as Arc<dyn ChatModel<()>>,
                    model_id: "managed-test-model".into(),
                    endpoint: "https://managed.test/openai/v1".into(),
                    credential: "frozen-managed-test-token".into(),
                })
            })
        },
    );
    register_inspection_chat_model_in(&extensions, "openhuman", factory);
    register_chat_model_in(
        &extensions,
        "openhuman",
        ChatModelEntry {
            build: Arc::new(move |request| {
                Ok((
                    Arc::new(CountingModel(Arc::new(AtomicUsize::new(0))))
                        as Arc<dyn ChatModel<()>>,
                    request
                        .model_override
                        .or(request.default_model)
                        .unwrap_or("managed-test-model")
                        .to_owned(),
                ))
            }),
            probe: None,
            models: None,
        },
    );

    let mut fixture = isolated_config();
    let config = &mut fixture.config;
    config.chat_provider = Some("openhuman".into());
    config.default_model = Some("managed-test-model".into());
    let Fixture { config, _temp } = fixture;
    let context = CoreContext::for_test_with_extensions(
        DomainSet::full(),
        Some(config.workspace_dir.clone()),
        extensions,
    );
    let report = CoreContext::scope(context, async move {
        crate::doctor::runner::run(
            &DoctorCtx::from_config(config),
            DoctorOptions {
                tier: DoctorTier::Live,
                only: vec![CHECK_CHAT.into()],
                ..DoctorOptions::default()
            },
        )
        .await
    })
    .await
    .expect("managed inspection callback completes");
    drop(_temp);

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(finding(&report, CHECK_CHAT).severity, FindingSeverity::Pass);
}

#[tokio::test]
async fn live_chat_maps_auth_quota_and_unavailable_errors_without_provider_text() {
    let cases = [(401, "auth"), (429, "quota"), (503, "unavailable")];
    for (status, classification) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(completion_path()))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "error": {"message": RESPONSE_SECRET, "code": "secret-provider-code"}
            })))
            .mount(&server)
            .await;

        let report = run_live(
            config_for(&server, "provider/configured-model"),
            vec![CHECK_CHAT],
        )
        .await;
        let result = finding(&report, CHECK_CHAT);
        assert_eq!(result.severity, FindingSeverity::Error, "HTTP {status}");
        assert!(
            result.message.to_ascii_lowercase().contains(classification),
            "{}",
            result.message
        );
        let serialized = serde_json::to_string(&report).expect("report serializes");
        for secret in [
            TEST_BEARER,
            ENDPOINT_SECRET,
            RESPONSE_SECRET,
            "secret-provider-code",
        ] {
            assert!(
                !serialized.contains(secret),
                "HTTP {status} leaked {secret}"
            );
        }
    }
}

#[tokio::test]
async fn identical_role_routes_share_one_completion_but_keep_each_role_finding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(completion_path()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(completion_response("provider/shared-model", "OK")),
        )
        .mount(&server)
        .await;
    let mut config = config_for(&server, "provider/shared-model");
    config.config.reasoning_provider = config.config.chat_provider.clone();

    let report = run_live(config, vec![CHECK_CHAT, "inference/live/reasoning"]).await;
    assert_eq!(finding(&report, CHECK_CHAT).severity, FindingSeverity::Pass);
    assert_eq!(
        finding(&report, "inference/live/reasoning").severity,
        FindingSeverity::Pass
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("mock records calls")
            .len(),
        1,
        "matching endpoint, credential, and model are probed once"
    );
}

#[tokio::test]
async fn same_endpoint_and_credential_with_distinct_models_get_distinct_completions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(completion_path()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(completion_response("provider/answered-model", "OK")),
        )
        .mount(&server)
        .await;
    let mut fixture = config_for(&server, "provider/chat-model");
    fixture.config.reasoning_provider = Some("ephemeral-route:reasoning-model".into());

    let report = run_live(fixture, vec![CHECK_CHAT, "inference/live/reasoning"]).await;
    assert_eq!(finding(&report, CHECK_CHAT).severity, FindingSeverity::Pass);
    assert_eq!(
        finding(&report, "inference/live/reasoning").severity,
        FindingSeverity::Pass
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("mock records calls")
            .len(),
        2,
        "a distinct resolved model needs its own completion"
    );
}

#[tokio::test]
async fn same_endpoint_and_model_with_distinct_credentials_get_distinct_completions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(provider_completion_path("shared")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(completion_response("provider/answered-model", "OK")),
        )
        .mount(&server)
        .await;
    let fixture = cloud_route_fixture(
        &server,
        &[
            (
                "route-one",
                "shared",
                "provider/shared-model",
                "dummy-key-one",
            ),
            (
                "route-two",
                "shared",
                "provider/shared-model",
                "dummy-key-two",
            ),
        ],
        "route-one",
        "route-two",
    );

    let report = run_live(fixture, vec![CHECK_CHAT, "inference/live/reasoning"]).await;
    assert_eq!(finding(&report, CHECK_CHAT).severity, FindingSeverity::Pass);
    assert_eq!(
        finding(&report, "inference/live/reasoning").severity,
        FindingSeverity::Pass
    );
    let requests = server
        .received_requests()
        .await
        .expect("mock records calls");
    assert_eq!(
        requests.len(),
        2,
        "different frozen credentials cannot deduplicate"
    );
    let mut authorization = requests
        .iter()
        .map(|request| {
            request
                .headers
                .get("authorization")
                .expect("credential reaches local mock")
                .to_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    authorization.sort();
    assert_eq!(
        authorization,
        vec!["Bearer dummy-key-one", "Bearer dummy-key-two"]
    );
}

#[tokio::test]
async fn distinct_endpoints_with_same_credential_and_model_get_distinct_completions() {
    let server = MockServer::start().await;
    for endpoint in ["first", "second"] {
        Mock::given(method("POST"))
            .and(path(provider_completion_path(endpoint)))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(completion_response("provider/answered-model", "OK")),
            )
            .mount(&server)
            .await;
    }
    let fixture = cloud_route_fixture(
        &server,
        &[
            (
                "route-one",
                "first",
                "provider/shared-model",
                "dummy-same-key",
            ),
            (
                "route-two",
                "second",
                "provider/shared-model",
                "dummy-same-key",
            ),
        ],
        "route-one",
        "route-two",
    );

    let report = run_live(fixture, vec![CHECK_CHAT, "inference/live/reasoning"]).await;
    assert_eq!(finding(&report, CHECK_CHAT).severity, FindingSeverity::Pass);
    assert_eq!(
        finding(&report, "inference/live/reasoning").severity,
        FindingSeverity::Pass
    );
    let requests = server
        .received_requests()
        .await
        .expect("mock records calls");
    assert_eq!(
        requests.len(),
        2,
        "different endpoints need separate probes"
    );
    let mut paths = requests
        .iter()
        .map(|request| request.url.path().to_string())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            provider_completion_path("first"),
            provider_completion_path("second")
        ]
    );
}

#[tokio::test]
async fn distinct_provider_slugs_with_the_same_endpoint_credential_and_model_deduplicate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(provider_completion_path("shared")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(completion_response("provider/answered-model", "OK")),
        )
        .mount(&server)
        .await;
    let fixture = cloud_route_fixture(
        &server,
        &[
            (
                "alias-one",
                "shared",
                "provider/shared-model",
                "dummy-same-key",
            ),
            (
                "alias-two",
                "shared",
                "provider/shared-model",
                "dummy-same-key",
            ),
        ],
        "alias-one",
        "alias-two",
    );

    let report = run_live(fixture, vec![CHECK_CHAT, "inference/live/reasoning"]).await;
    assert_eq!(finding(&report, CHECK_CHAT).severity, FindingSeverity::Pass);
    assert_eq!(
        finding(&report, "inference/live/reasoning").severity,
        FindingSeverity::Pass
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("mock records calls")
            .len(),
        1,
        "provider ids and slugs are not part of the effective route identity"
    );
}

#[tokio::test]
async fn connect_tier_skips_live_chat_without_sending_a_completion() {
    let server = MockServer::start().await;
    let fixture = config_for(&server, "provider/model");
    let Fixture { config, _temp } = fixture;
    let context = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    let report = CoreContext::scope(context, async move {
        crate::doctor::runner::run(
            &DoctorCtx::from_config(config),
            DoctorOptions {
                tier: DoctorTier::Connect,
                only: vec![CHECK_CHAT.into()],
                ..DoctorOptions::default()
            },
        )
        .await
    })
    .await
    .expect("connect tier returns report");
    drop(_temp);

    assert_eq!(report.checks_skipped, vec![CHECK_CHAT]);
    assert!(report.findings.iter().any(|result| {
        result.check_id == CHECK_CHAT && result.severity == FindingSeverity::Skipped
    }));
    assert!(!report.findings.iter().any(|result| {
        result.check_id == "doctor/selection" && result.severity == FindingSeverity::Error
    }));
    assert!(server
        .received_requests()
        .await
        .expect("mock records calls")
        .is_empty());
}

#[test]
fn configured_role_ids_are_registered_as_live_checks_for_that_config() {
    use crate::inference::provider::factory::provider_for_role;

    let mut fixture = isolated_config();
    let config = &mut fixture.config;
    config.default_model = Some("shared-model".into());
    config.chat_provider = Some("ephemeral-route:shared-model".into());
    config.reasoning_provider = Some("ephemeral-route:shared-model".into());
    config.vision_provider = Some("ephemeral-route:vision-model".into());
    config.memory_provider = Some("ephemeral-route:summary-model".into());
    let ctx = DoctorCtx::from_config(config.clone());
    let checks = crate::doctor::registry::registered_checks_for(&ctx)
        .expect("selected config supplies its own live role checks");
    let by_id = checks
        .iter()
        .map(|check| (check.id().to_owned(), check.tier()))
        .collect::<std::collections::HashMap<_, _>>();

    for role in [
        "chat",
        "reasoning",
        "agentic",
        "burst",
        "coding",
        "vision",
        "memory",
        "summarization",
    ] {
        let id = format!("inference/live/{role}");
        assert_eq!(by_id.get(&id), Some(&DoctorTier::Live), "missing {id}");
        assert!(!provider_for_role(role, config).is_empty());
    }
    assert!(!by_id.contains_key("inference/live/embeddings"));
}

#[path = "inference_live_identity_tests.rs"]
mod identity_tests;
