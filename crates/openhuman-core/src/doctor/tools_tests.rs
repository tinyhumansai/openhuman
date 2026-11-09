use super::*;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use crate::core::slots::Extensions;
use crate::tools::toolpacks::ToolGroups;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tinytools::{PermissionLevel, ToolScope};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

fn config_in(tmp: &TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    let action_dir = tmp.path().join("action");
    std::fs::create_dir_all(&workspace).expect("create test workspace");
    std::fs::create_dir_all(&action_dir).expect("create test action directory");

    Config {
        workspace_dir: workspace.clone(),
        action_dir,
        config_path: workspace.join("config.toml"),
        ..Config::default()
    }
}

fn scoped_context(config: &Config, extensions: Extensions) -> Arc<CoreContext> {
    let parent = CoreContext::for_test_with_extensions(
        DomainSet::full(),
        Some(config.workspace_dir.clone()),
        extensions,
    );
    parent.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::none())
            .without_user_skill_roots(),
    )
}

#[test]
fn metadata() {
    assert_eq!(DoctorHealthTool::new(cfg()).name(), "doctor_health");
    assert_eq!(DoctorModelsTool::new(cfg()).name(), "doctor_models");
    assert_eq!(
        DoctorHealthTool::new(cfg()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(DoctorHealthTool::new(cfg()).scope(), ToolScope::All);
}

#[test]
fn health_schema_exposes_only_optional_tier_and_exact_id_selection() {
    let tool = DoctorHealthTool::new(cfg());
    let schema = tool.parameters_schema();

    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["properties"]["tier"]["enum"],
        serde_json::json!(["static", "connect", "live"])
    );
    assert_eq!(schema["properties"]["tier"]["default"], "connect");
    assert_eq!(schema["properties"]["only"]["type"], "array");
    assert_eq!(schema["properties"]["only"]["items"]["type"], "string");
    assert!(
        schema["required"].is_null(),
        "both inputs should be optional"
    );

    let description = format!(
        "{} {}",
        tool.description(),
        schema["properties"]["tier"]["description"]
            .as_str()
            .expect("tier description")
    )
    .to_ascii_lowercase();
    assert!(description.contains("live"), "{description}");
    assert!(description.contains("token"), "{description}");
    assert!(
        description.contains("user explicitly requested"),
        "{description}"
    );
}

#[tokio::test]
async fn models_tool_returns_an_inspection_report_when_no_provider_is_configured() {
    DoctorModelsTool::new(cfg())
        .execute(serde_json::json!({ "use_cache": true }))
        .await
        .expect("doctor_models returns the actual empty inspection result");
}

#[tokio::test]
async fn health_tool_forwards_tier_and_only_to_the_scoped_check_registry() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let selected_calls = Arc::new(AtomicUsize::new(0));
    let unselected_calls = Arc::new(AtomicUsize::new(0));
    let extensions = Extensions::new();
    crate::doctor::register_check_source_in(
        &extensions,
        crate::doctor::test_support::FakeCheckSource::new(
            "doctor-health-tool-forwarding",
            vec![
                crate::doctor::test_support::FakeCheck::pass("test/selected")
                    .with_tier(crate::doctor::DoctorTier::Static)
                    .with_call_counter(Arc::clone(&selected_calls)),
                crate::doctor::test_support::FakeCheck::pass("test/unselected")
                    .with_tier(crate::doctor::DoctorTier::Static)
                    .with_call_counter(Arc::clone(&unselected_calls)),
            ],
        ),
    );
    let context = scoped_context(&config, extensions);
    let tool = DoctorHealthTool::new(Arc::new(config));

    let result = CoreContext::scope(
        context,
        tool.execute(serde_json::json!({
            "tier": "static",
            "only": ["test/selected"]
        })),
    )
    .await
    .expect("the scoped static check should return a report");
    let report: serde_json::Value =
        serde_json::from_str(&result.text()).expect("tool result is a JSON doctor report");

    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["tier"], "static");
    assert_eq!(report["checks_run"], serde_json::json!(["test/selected"]));
    assert_eq!(selected_calls.load(Ordering::SeqCst), 1);
    assert_eq!(unselected_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn health_tool_defaults_to_connect_when_tier_is_omitted() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let selected_calls = Arc::new(AtomicUsize::new(0));
    let extensions = Extensions::new();
    crate::doctor::register_check_source_in(
        &extensions,
        crate::doctor::test_support::FakeCheckSource::new(
            "doctor-health-tool-default-tier",
            vec![
                crate::doctor::test_support::FakeCheck::pass("test/default-connect")
                    .with_tier(crate::doctor::DoctorTier::Connect)
                    .with_call_counter(Arc::clone(&selected_calls)),
            ],
        ),
    );
    let context = scoped_context(&config, extensions);
    let tool = DoctorHealthTool::new(Arc::new(config));

    let result = CoreContext::scope(
        context,
        tool.execute(serde_json::json!({
            "only": ["test/default-connect"]
        })),
    )
    .await
    .expect("the selected connect check should return a report");
    let report: serde_json::Value =
        serde_json::from_str(&result.text()).expect("tool result is a JSON doctor report");

    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["tier"], "connect");
    assert_eq!(
        report["checks_run"],
        serde_json::json!(["test/default-connect"])
    );
    assert_eq!(selected_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn models_tool_reports_each_configured_catalog_outcome() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/available/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{ "id": "fixture-model" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/missing/v1/models"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_in(&tmp);
    config.default_model = None;
    config.model_routes.clear();
    config.cloud_providers = vec![
        crate::config::schema::cloud_providers::CloudProviderCreds {
            id: "p_doctor_tool_models_available".into(),
            slug: "doctor-tool-models-available".into(),
            label: "Available test provider".into(),
            endpoint: format!("{}/available/v1", server.uri()),
            auth_style: crate::config::schema::cloud_providers::AuthStyle::None,
            default_model: None,
        },
        crate::config::schema::cloud_providers::CloudProviderCreds {
            id: "p_doctor_tool_models_missing".into(),
            slug: "doctor-tool-models-missing".into(),
            label: "Missing test provider".into(),
            endpoint: format!("{}/missing/v1", server.uri()),
            auth_style: crate::config::schema::cloud_providers::AuthStyle::None,
            default_model: None,
        },
    ];

    let result = DoctorModelsTool::new(Arc::new(config))
        .execute(serde_json::json!({ "use_cache": false }))
        .await
        .expect("catalog outcomes should be returned as a report");
    let report: serde_json::Value =
        serde_json::from_str(&result.text()).expect("tool result is a JSON model probe report");
    let entries = report["entries"].as_array().expect("provider entries");
    let available = entries
        .iter()
        .find(|entry| entry["provider"] == "doctor-tool-models-available")
        .expect("available provider entry");
    let missing = entries
        .iter()
        .find(|entry| entry["provider"] == "doctor-tool-models-missing")
        .expect("missing provider entry");

    assert_eq!(available["outcome"], "Ok");
    assert_eq!(available["message"], "Catalog returned 1 models.");
    assert_eq!(missing["outcome"], "Skipped");
    assert_eq!(
        missing["message"],
        "Provider does not expose a models catalog."
    );
    assert_eq!(report["summary"]["ok"], 1);
    assert_eq!(report["summary"]["skipped"], 1);
    assert_eq!(report["summary"]["errors"], 0);
}

#[test]
fn gitbook_inventory_lists_every_registered_doctor_check_id() {
    let docs_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/gitbooks/en/guides/doctor.md");
    let docs = std::fs::read_to_string(&docs_path).unwrap_or_else(|error| {
        panic!(
            "doctor check inventory page should exist at {}: {error}",
            docs_path.display()
        )
    });
    let checks = crate::doctor::registry::registered_checks()
        .expect("registered doctor checks should have valid unique ids");

    for check in checks {
        let documented_id = format!("`{}`", check.id());
        assert!(
            docs.contains(&documented_id),
            "registered doctor check `{}` is missing from {}",
            check.id(),
            docs_path.display()
        );
    }
}
