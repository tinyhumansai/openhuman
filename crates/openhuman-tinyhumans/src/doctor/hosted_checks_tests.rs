use super::*;
use crate::session::test_support::{EXPIRED_JWT, LIVE_JWT};
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::core::runtime::CoreContext;
use openhuman_embed::__host::doctor::{registered_checks, ComponentId, DoctorCheck};
use openhuman_embed::{CoreBuilder, DomainSet, HostKind, ServiceSet};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn context(tmp: &TempDir, api_url: &str) -> DoctorCtx {
    let state_dir = tmp.path().join("workspace");
    context_at(&state_dir, &state_dir, api_url)
}

fn config_at(state_dir: &Path, workspace_dir: &Path, api_url: &str) -> Config {
    let mut config = Config {
        workspace_dir: workspace_dir.to_path_buf(),
        action_dir: workspace_dir.to_path_buf(),
        config_path: state_dir.join("config.toml"),
        api_url: Some(api_url.to_owned()),
        ..Config::default()
    };
    config.resolved_endpoints.control_plane_url = Some(api_url.to_owned());
    config
}

fn context_at(state_dir: &Path, workspace_dir: &Path, api_url: &str) -> DoctorCtx {
    DoctorCtx::from_config(config_at(state_dir, workspace_dir, api_url))
}

fn write_raw_session_profile(
    ctx: &DoctorCtx,
    token: &str,
    metadata: serde_json::Map<String, serde_json::Value>,
) -> (PathBuf, Vec<u8>) {
    let state_dir = ctx.config().config_path.parent().expect("config state dir");
    std::fs::create_dir_all(state_dir).expect("create temp config state dir");
    let path = state_dir.join("auth-profiles.json");
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "updated_at": "2026-01-01T00:00:00Z",
        "active_profiles": {},
        "profiles": {
            "app-session:default": {
                "provider": "app-session",
                "profile_name": "default",
                "kind": "token",
                "token": token,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "metadata": metadata,
            }
        }
    }))
    .expect("serialize schema-1 auth profile");
    std::fs::write(&path, &bytes).expect("write schema-1 auth profile");
    (path, bytes)
}

fn metadata(
    entries: impl IntoIterator<Item = (&'static str, String)>,
) -> serde_json::Map<String, serde_json::Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), serde_json::Value::String(value)))
        .collect()
}

fn profile_store_snapshot(workspace: &Path) -> Vec<u8> {
    std::fs::read(workspace.join("auth-profiles.json"))
        .expect("read session profile store snapshot")
}

async fn registered_ids(runtime: &openhuman_embed::CoreRuntime) -> Vec<String> {
    CoreContext::scope(runtime.context().clone(), async {
        registered_checks()
            .expect("registered checks")
            .iter()
            .map(|check| check.id().to_owned())
            .collect::<Vec<_>>()
    })
    .await
}

fn check(source: &HostedDoctorCheckSource, id: &str) -> Arc<dyn DoctorCheck> {
    source
        .checks()
        .into_iter()
        .find(|check| check.id() == id)
        .unwrap_or_else(|| panic!("missing hosted doctor check {id}"))
}

async fn findings(source: &HostedDoctorCheckSource, ctx: &DoctorCtx) -> Vec<Finding> {
    let checks = source.checks();
    let mut findings = Vec::with_capacity(checks.len());
    for check in checks {
        findings.extend(check.detect(ctx).await);
    }
    findings
}

async fn run_isolated_probe(scenario: &str, root: &Path, api_url: &str) -> serde_json::Value {
    let workspace = root.join("workspace");
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "doctor::tests::hosted_probe_child",
            "--nocapture",
        ])
        .env("OPENHUMAN_DOCTOR_PROBE_CHILD_SCENARIO", scenario)
        .env("OPENHUMAN_DOCTOR_PROBE_CHILD_ROOT", root)
        .env("OPENHUMAN_DOCTOR_PROBE_CHILD_API_URL", api_url)
        .env("OPENHUMAN_WORKSPACE", &workspace)
        .env("OPENHUMAN_KEYRING_BACKEND", "encrypted_file")
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "11".repeat(32))
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .output()
        .await
        .expect("run isolated doctor child test");
    assert!(
        output.status.success(),
        "child test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&std::fs::read(root.join("child-result.json")).expect("child report"))
        .expect("parse child report")
}

#[tokio::test]
async fn hosted_probe_child() {
    let Some(scenario) = std::env::var_os("OPENHUMAN_DOCTOR_PROBE_CHILD_SCENARIO") else {
        return;
    };
    let scenario = scenario.to_string_lossy();
    let root =
        PathBuf::from(std::env::var_os("OPENHUMAN_DOCTOR_PROBE_CHILD_ROOT").expect("child root"));
    openhuman_embed::process::init_master_key().expect("initialize isolated test key source");
    openhuman_embed::__host::core::bus::init()
        .await
        .expect("initialize isolated child bus");
    let mut events = openhuman_embed::__host::core::bus::BUS
        .get()
        .expect("initialized bus")
        .receiver();
    let workspace = root.join("workspace");
    let api_url = std::env::var("OPENHUMAN_DOCTOR_PROBE_CHILD_API_URL").expect("child api url");
    let ctx = context_at(&workspace, &workspace, &api_url);
    let (active_url, active_token, active_status) = match scenario.as_ref() {
        "socket-mismatch" => (
            "https://profile-a.example/socket.io".to_owned(),
            "session-a".to_owned(),
            ConnectionStatus::Connected,
        ),
        "socket-owner-error" => (
            api_url.clone(),
            LIVE_JWT.to_string(),
            ConnectionStatus::Error,
        ),
        "socket-owner-reconnecting" => (
            api_url.clone(),
            LIVE_JWT.to_string(),
            ConnectionStatus::Reconnecting,
        ),
        _ => (
            api_url.clone(),
            LIVE_JWT.to_string(),
            ConnectionStatus::Connected,
        ),
    };
    let source = HostedDoctorCheckSource::with_socket_state_reader(move |url, token| {
        (url == active_url && token == active_token).then_some(active_status)
    });
    let before = (scenario == "expired").then(|| profile_store_snapshot(&workspace));

    let report = match scenario.as_ref() {
        "healthy" => {
            let results = findings(&source, &ctx).await;
            json!({
                "findings": results.iter().map(|finding| json!({
                    "check_id": finding.check_id,
                    "severity": format!("{:?}", finding.severity),
                    "message": finding.message,
                })).collect::<Vec<_>>(),
            })
        }
        "outage" => {
            let results = [
                "backend/reachable",
                "backend/identity",
                "inference/managed-catalog",
            ]
            .into_iter()
            .map(|id| check(&source, id));
            let mut findings = Vec::new();
            for probe in results {
                findings.extend(probe.detect(&ctx).await);
            }
            json!({
                "findings": findings.iter().map(|finding| json!({
                    "check_id": finding.check_id,
                    "severity": format!("{:?}", finding.severity),
                    "message": finding.message,
                })).collect::<Vec<_>>(),
            })
        }
        "issuer-mismatch" => {
            let results = [
                "backend/credential",
                "backend/identity",
                "inference/managed-catalog",
            ]
            .into_iter()
            .map(|id| check(&source, id));
            let mut findings = Vec::new();
            for probe in results {
                findings.extend(probe.detect(&ctx).await);
            }
            json!({
                "findings": findings.iter().map(|finding| json!({
                    "check_id": finding.check_id,
                    "severity": format!("{:?}", finding.severity),
                    "message": finding.message,
                })).collect::<Vec<_>>(),
            })
        }
        "socket-mismatch" | "socket-owner-error" | "socket-owner-reconnecting" => {
            let finding = check(&source, "backend/socket").detect(&ctx).await;
            json!({
                "severity": format!("{:?}", finding[0].severity),
                "message": finding[0].message,
            })
        }
        "scoped-context" => {
            let config = config_at(&root, &workspace, &api_url);
            let hosted = crate::RuntimeBuilder::new()
                .workspace(crate::embed::Workspace::Dir(workspace.clone()))
                .services(crate::embed::ServiceSet::none())
                .domains(crate::embed::DomainSet::none())
                .config(config.clone())
                .build()
                .await
                .expect("TinyHumans runtime context");
            let core_only = CoreBuilder::new(HostKind::Library)
                .services(ServiceSet::none())
                .domains(DomainSet::none())
                .config(config)
                .build()
                .await
                .expect("plain core context");
            json!({
                "hosted_ids": registered_ids(hosted.core_runtime()).await,
                "core_ids": registered_ids(&core_only).await,
            })
        }
        "expired" => {
            let finding = check(&source, "backend/credential").detect(&ctx).await;
            let mut session_expired_published = false;
            while let Ok(event) = events.try_recv() {
                if matches!(
                    event,
                    openhuman_embed::__host::core::events::DomainEvent::SessionExpired { .. }
                ) {
                    session_expired_published = true;
                }
            }
            json!({
                "severity": format!("{:?}", finding[0].severity),
                "message": finding[0].message,
                "profile_store_unchanged": before.as_ref() == Some(&profile_store_snapshot(&workspace)),
                "session_expired_published": session_expired_published,
            })
        }
        _ => panic!("unknown child scenario"),
    };
    std::fs::write(
        root.join("child-result.json"),
        serde_json::to_vec(&report).unwrap(),
    )
    .expect("write child result outside profile workspace");
}

#[tokio::test]
async fn healthy_backend_passes_hosted_checks_and_uses_sdk_attribution() {
    let _registry = crate::registry_test_lock_async().await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("x-sdk-name", "openhuman"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth/me"))
        .and(header("x-sdk-name", "openhuman"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": {"user": {"id": "private-user"}}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/openai/v1/models"))
        .and(query_param("catalog", "openrouter"))
        .and(header("x-sdk-name", "openhuman"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let ctx = context(&tmp, &server.uri());
    write_raw_session_profile(
        &ctx,
        &LIVE_JWT,
        metadata([("session_issuing_backend", server.uri())]),
    );
    let report = run_isolated_probe("healthy", tmp.path(), &server.uri()).await;
    let findings = report["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 6);
    assert!(findings.iter().all(|finding| finding["severity"] == "Pass"));
    assert!(findings.iter().all(|finding| {
        !finding["message"]
            .as_str()
            .unwrap()
            .contains("private-user")
    }));
    let requests = server.received_requests().await.unwrap();
    assert!(requests.iter().any(|request| request.url.path() == "/"));
    assert!(requests
        .iter()
        .any(|request| request.url.path() == "/auth/me"));
    assert!(requests
        .iter()
        .any(|request| request.url.path() == "/openai/v1/models"));
    assert!(requests
        .iter()
        .all(|request| request.headers.get("x-sdk-name").is_some()));
}

#[tokio::test]
async fn backend_outage_skips_authenticated_hosted_probes_without_leaking_response_data() {
    let _registry = crate::registry_test_lock_async().await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(503).set_body_string("token=private-token user=private-user"),
        )
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let ctx = context(&tmp, &server.uri());
    write_raw_session_profile(
        &ctx,
        &LIVE_JWT,
        metadata([("session_issuing_backend", server.uri())]),
    );
    let report = run_isolated_probe("outage", tmp.path(), &server.uri()).await;
    let findings = report["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 3);
    assert_eq!(findings[0]["severity"], "Error");
    assert_eq!(findings[1]["severity"], "Skipped");
    assert_eq!(findings[2]["severity"], "Skipped");
    assert!(findings.iter().all(|finding| {
        !finding["message"]
            .as_str()
            .unwrap()
            .contains("private-token")
    }));
    assert!(findings.iter().all(|finding| {
        !finding["message"]
            .as_str()
            .unwrap()
            .contains("private-user")
    }));
    let requests = server.received_requests().await.unwrap();
    assert!(requests.iter().all(|request| request.url.path() == "/"));
    assert!(requests
        .iter()
        .all(|request| !request.headers.contains_key("authorization")));
}

#[tokio::test]
async fn expired_session_probe_preserves_profile_and_does_not_publish_expiry() {
    let _registry = crate::registry_test_lock_async().await;
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    let ctx = context_at(&workspace, &workspace, "http://127.0.0.1:9");
    write_raw_session_profile(
        &ctx,
        &EXPIRED_JWT,
        metadata([
            ("session_issuing_backend", "http://127.0.0.1:9".to_owned()),
            ("session_expires_at", "2001-01-01T00:00:00Z".to_owned()),
        ]),
    );

    let report = run_isolated_probe("expired", tmp.path(), "http://127.0.0.1:9").await;
    assert_eq!(report["severity"], "Error");
    assert!(report["message"]
        .as_str()
        .unwrap()
        .contains("SESSION_EXPIRED"));
    assert_eq!(report["profile_store_unchanged"], true);
    assert_eq!(report["session_expired_published"], false);
    assert!(!report["message"].as_str().unwrap().contains(&*EXPIRED_JWT));
}

#[tokio::test]
async fn foreign_socket_connection_is_skipped_for_selected_profile() {
    let tmp = TempDir::new().unwrap();
    let api_url = "https://selected-b.example";
    let ctx = context(&tmp, api_url);
    write_raw_session_profile(
        &ctx,
        &LIVE_JWT,
        metadata([("session_issuing_backend", api_url.to_owned())]),
    );
    let report = run_isolated_probe("socket-mismatch", tmp.path(), api_url).await;
    assert_eq!(report["severity"], "Skipped");
    assert!(!report["message"].as_str().unwrap().contains(&*LIVE_JWT));
}

#[tokio::test]
async fn socket_error_for_selected_profile_remains_an_error() {
    let tmp = TempDir::new().unwrap();
    let api_url = "https://selected-b.example";
    let ctx = context(&tmp, api_url);
    write_raw_session_profile(
        &ctx,
        &LIVE_JWT,
        metadata([("session_issuing_backend", api_url.to_owned())]),
    );
    let report = run_isolated_probe("socket-owner-error", tmp.path(), api_url).await;
    assert_eq!(report["severity"], "Error");
}

#[tokio::test]
async fn reconnecting_socket_status_is_preserved_for_its_owner() {
    let tmp = TempDir::new().unwrap();
    let api_url = "https://selected-b.example";
    let ctx = context(&tmp, api_url);
    write_raw_session_profile(
        &ctx,
        &LIVE_JWT,
        metadata([("session_issuing_backend", api_url.to_owned())]),
    );
    let report = run_isolated_probe("socket-owner-reconnecting", tmp.path(), api_url).await;
    assert_eq!(report["severity"], "Warn");
}

#[tokio::test]
async fn mismatched_issuing_backend_is_rejected_before_authenticated_probes() {
    let _registry = crate::registry_test_lock_async().await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("x-sdk-name", "openhuman"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"user": "private-user"})))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/openai/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .expect(0)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let ctx = context(&tmp, &server.uri());
    write_raw_session_profile(
        &ctx,
        &LIVE_JWT,
        metadata([(
            "session_issuing_backend",
            "https://issuer-a.example".to_owned(),
        )]),
    );
    let report = run_isolated_probe("issuer-mismatch", tmp.path(), &server.uri()).await;
    let findings = report["findings"].as_array().unwrap();
    assert_eq!(findings[0]["severity"], "Error");
    assert!(findings[0]["message"]
        .as_str()
        .unwrap()
        .contains("SESSION_BACKEND_MISMATCH"));
    assert_eq!(findings[1]["severity"], "Skipped");
    assert_eq!(findings[2]["severity"], "Skipped");
    for finding in findings {
        assert!(!finding["message"].as_str().unwrap().contains(&*LIVE_JWT));
        assert!(!finding["message"]
            .as_str()
            .unwrap()
            .contains("issuer-a.example"));
        assert!(!finding["message"]
            .as_str()
            .unwrap()
            .contains("private-user"));
    }

    let requests = server.received_requests().await.unwrap();
    assert!(!requests
        .iter()
        .any(|request| request.url.path() == "/auth/me"));
    assert!(!requests
        .iter()
        .any(|request| request.url.path() == "/openai/v1/models"));
    assert!(requests.iter().all(|request| request.url.path() == "/"));
    assert!(requests
        .iter()
        .all(|request| !request.headers.contains_key("authorization")));
    server.verify().await;
}

#[test]
fn backend_credential_check_runs_in_the_connect_tier() {
    assert_eq!(
        check(&HostedDoctorCheckSource::default(), "backend/credential").tier(),
        DoctorTier::Connect
    );
}

#[test]
fn every_hosted_check_is_gated_by_the_hosting_component() {
    let checks = HostedDoctorCheckSource::default().checks();
    let expected_ids = [
        "backend/reachable",
        "backend/credential",
        "backend/identity",
        "backend/socket",
        "backend/api-url",
        "inference/managed-catalog",
    ];

    assert_eq!(
        checks.iter().map(|check| check.id()).collect::<Vec<_>>(),
        expected_ids
    );
    for check in checks {
        assert_eq!(
            check.component_gate(),
            Some(ComponentId::Hosting),
            "{} must require the Hosting component",
            check.id()
        );
    }
}

#[tokio::test]
async fn tinyhumans_runtime_registration_is_context_scoped() {
    let tmp = TempDir::new().unwrap();
    let api_url = "https://scope-test.example";
    let report = run_isolated_probe("scoped-context", tmp.path(), api_url).await;
    let hosted_ids = report["hosted_ids"].as_array().unwrap();
    let core_ids = report["core_ids"].as_array().unwrap();

    for id in [
        "backend/reachable",
        "backend/credential",
        "backend/identity",
        "backend/socket",
        "backend/api-url",
        "inference/managed-catalog",
    ] {
        assert!(hosted_ids.iter().any(|registered| registered == id), "{id}");
        assert!(!core_ids.iter().any(|registered| registered == id), "{id}");
    }
}
