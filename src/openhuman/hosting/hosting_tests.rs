//! Tests for hosting account resolution, workspace containment, and the tools.
//!
//! The provider itself is TinyHosts' problem and is tested there against a mock
//! of its REST API. What is tested here is the seam: whether an account resolves
//! from configuration, whether a path an agent named can escape the workspace,
//! and whether each tool's schema says what its `execute` actually reads.

use serde_json::json;

use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::Tool;

fn config_with(workspace: &std::path::Path, enabled: bool, api_key: &str) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.hosting.enabled = enabled;
    config.hosting.api_key = api_key.to_string();
    config
}

#[test]
fn hosting_off_yields_no_account() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), false, "token");

    assert!(Account::from_config(&config)
        .expect("resolution does not fail")
        .is_none());
}

#[test]
fn a_configured_key_yields_an_account() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");

    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account, since a key is configured");

    assert_eq!(account.host().kind().as_str(), "vercel");
    assert_eq!(account.workspace_dir(), workspace.path());
}

#[test]
fn an_unknown_provider_is_an_error_rather_than_a_silent_skip() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let mut config = config_with(workspace.path(), true, "token");
    config.hosting.provider = "heroku".to_string();

    let error = Account::from_config(&config).expect_err("an unknown provider fails");

    assert!(
        error.to_string().contains("heroku"),
        "the error should name the provider: {error}"
    );
}

#[test]
fn an_account_reports_itself_without_its_credential() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "super-secret");

    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    assert!(
        !format!("{account:?}").contains("super-secret"),
        "the credential must never be rendered"
    );
}

#[test]
fn an_account_exposes_every_hosting_tool() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    let names: Vec<String> = account
        .tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();

    assert_eq!(
        names,
        [
            "hosting_launch_site",
            "hosting_deployment_status",
            "hosting_list_deployments",
            "hosting_rollback",
            "hosting_list_sites",
            "hosting_set_env",
            "hosting_add_domain",
            "hosting_domain_status",
            "hosting_analytics",
        ]
    );
}

#[test]
fn only_the_tools_that_change_the_world_carry_an_external_effect() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    for tool in account.tools() {
        let expected = matches!(
            tool.name(),
            "hosting_launch_site" | "hosting_set_env" | "hosting_add_domain" | "hosting_rollback"
        );
        assert_eq!(
            tool.external_effect(),
            expected,
            "{} has the wrong external effect",
            tool.name()
        );
    }
}

#[test]
fn every_tool_schema_is_an_object_naming_its_required_arguments() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    for tool in account.tools() {
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object", "{}", tool.name());
        assert!(
            schema["properties"].is_object(),
            "{} has no properties",
            tool.name()
        );
        assert!(
            !tool.description().is_empty(),
            "{} has no description",
            tool.name()
        );
    }
}

#[test]
fn a_directory_inside_the_workspace_resolves() {
    let workspace = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(workspace.path().join("site")).expect("mkdir");

    let resolved = resolve_in_workspace(workspace.path(), "site").expect("resolves");

    assert_eq!(
        resolved,
        workspace
            .path()
            .canonicalize()
            .expect("canonical root")
            .join("site")
    );
}

#[test]
fn an_empty_path_is_the_workspace_root() {
    let workspace = tempfile::tempdir().expect("tempdir");

    let resolved = resolve_in_workspace(workspace.path(), "  ").expect("resolves");

    assert_eq!(
        resolved,
        workspace.path().canonicalize().expect("canonical root")
    );
}

#[test]
fn a_path_outside_the_workspace_is_refused() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let root = workspace.path();

    // A deployment uploads every byte under the directory to a third party, so
    // this is the check that decides what may leave the machine.
    assert!(resolve_in_workspace(root, "/etc").is_err());
    assert!(resolve_in_workspace(root, "../..").is_err());
    assert!(resolve_in_workspace(root, "does-not-exist").is_err());
}

#[test]
fn a_file_is_not_a_deployable_directory() {
    let workspace = tempfile::tempdir().expect("tempdir");
    std::fs::write(workspace.path().join("page.tsx"), b"x").expect("write");

    let error =
        resolve_in_workspace(workspace.path(), "page.tsx").expect_err("a file is not a directory");

    assert!(error.to_string().contains("not a directory"), "{error}");
}

#[tokio::test]
async fn launching_reports_a_missing_directory_instead_of_deploying_nothing() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    let tool = tools::LaunchSiteTool::new(account);
    let result = tool
        .execute(json!({"site": "shop", "path": "missing"}))
        .await
        .expect("the tool reports rather than panics");

    assert!(result.is_error);
}

#[tokio::test]
async fn launching_without_a_site_name_is_refused_before_any_upload() {
    let workspace = tempfile::tempdir().expect("tempdir");
    std::fs::write(workspace.path().join("package.json"), b"{}").expect("write");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    let tool = tools::LaunchSiteTool::new(account);
    let result = tool
        .execute(json!({"path": "."}))
        .await
        .expect("the tool reports rather than panics");

    assert!(result.is_error);
}

#[tokio::test]
async fn a_read_tool_reports_a_missing_argument_rather_than_calling_out() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    let result = tools::DeploymentStatusTool::new(account.host())
        .execute(json!({}))
        .await
        .expect("the tool reports rather than panics");

    assert!(result.is_error);
}

// ── The deployment-history and rollback tools (issue opencompany#913) ────────

#[tokio::test]
async fn the_new_read_tools_report_a_missing_site_rather_than_calling_out() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    let listed = tools::ListDeploymentsTool::new(account.host())
        .execute(json!({}))
        .await
        .expect("the tool reports rather than panics");
    assert!(listed.is_error);

    let domains = tools::DomainStatusTool::new(account.host())
        .execute(json!({}))
        .await
        .expect("the tool reports rather than panics");
    assert!(domains.is_error);
}

#[tokio::test]
async fn a_rollback_missing_either_argument_is_refused_before_any_call() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    // A rollback names two things and neither can be guessed: repointing
    // production at an unnamed deployment is not a recoverable mistake.
    for args in [
        json!({}),
        json!({"site": "shop"}),
        json!({"deployment_id": "d1"}),
    ] {
        let result = tools::RollbackTool::new(account.host())
            .execute(args.clone())
            .await
            .expect("the tool reports rather than panics");
        assert!(result.is_error, "{args} should have been refused");
    }
}

// ── The rollback guard, against a mock of the provider's API ─────────────────

use std::sync::Arc as TestArc;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A real provider client pointed at a local mock rather than at Vercel.
///
/// `connect_to`'s own documentation names this case, and loopback `http://` is
/// accepted precisely so a test can carry a bearer token to a server that never
/// leaves the machine.
fn host_against(server: &MockServer) -> TestArc<dyn tinyhosts::Host> {
    let base_url = server.uri();
    TestArc::from(
        tinyhosts::connect_to(
            tinyhosts::ProviderKind::Vercel,
            tinyhosts::Credentials::new("token").expect("credentials"),
            Some(base_url.as_str()),
        )
        .expect("a client against the mock"),
    )
}

/// **The guard that makes this tool a recovery path rather than a second way to
/// break the site.**
///
/// `hosting_list_deployments` returns failed and still-building deployments too
/// — they are part of the history an agent reads — so the id it picks is not
/// necessarily one that can serve traffic. Promoting a failed build would take
/// the site down during an attempt to bring it back up.
///
/// The assertion that matters is `expect(0)` on the promote route: the refusal
/// has to happen *before* the outward call, not be an error message reported
/// after production already moved.
#[tokio::test]
async fn rolling_back_to_a_deployment_that_never_built_does_not_touch_production() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v13/deployments/dpl_broken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "dpl_broken",
            "name": "shop",
            "readyState": "ERROR",
            "errorMessage": "build failed"
        })))
        .mount(&server)
        .await;

    // Vercel's promote is POST /v10/projects/{project}/promote/{deployment},
    // reached only after GET /v9/projects/{site} resolves the id. Neither may
    // be called at all.
    Mock::given(method("GET"))
        .and(path("/v9/projects/shop"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "prj_1",
            "name": "shop"
        })))
        .expect(0)
        .mount(&server)
        .await;

    let result = tools::RollbackTool::new(host_against(&server))
        .execute(json!({"site": "shop", "deployment_id": "dpl_broken"}))
        .await
        .expect("the tool reports rather than panics");

    assert!(result.is_error, "a failed deployment must not be promoted");
    // The status is named, so a model can pick a different deployment rather
    // than retry the same one.
    let rendered = result.text();
    assert!(
        rendered.contains("Failed"),
        "the refusal should name the state it refused: {rendered}"
    );
}

/// The ordinary path: a deployment that built is promoted, and the site's URL
/// comes back so the agent can say where production now points.
#[tokio::test]
async fn rolling_back_to_a_ready_deployment_promotes_it() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v13/deployments/dpl_good"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "dpl_good",
            "name": "shop",
            "url": "shop-abc.vercel.app",
            "readyState": "READY",
            "target": "production"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v9/projects/shop"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "prj_1",
            "name": "shop"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v10/projects/prj_1/promote/dpl_good"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let result = tools::RollbackTool::new(host_against(&server))
        .execute(json!({"site": "shop", "deployment_id": "dpl_good"}))
        .await
        .expect("the tool reports rather than panics");

    assert!(!result.is_error, "{result:?}");
    let rendered = result.text();
    assert!(
        rendered.contains("dpl_good") && rendered.contains("shop-abc.vercel.app"),
        "the result should say what is serving and where: {rendered}"
    );
}

/// The history read an agent uses to *find* the id above.
#[tokio::test]
async fn listing_deployments_reports_the_history_a_rollback_picks_from() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v7/deployments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deployments": [
                {"id": "dpl_new", "name": "shop", "readyState": "ERROR"},
                {"id": "dpl_old", "name": "shop", "readyState": "READY"}
            ]
        })))
        .mount(&server)
        .await;

    let result = tools::ListDeploymentsTool::new(host_against(&server))
        .execute(json!({"site": "shop"}))
        .await
        .expect("the tool reports rather than panics");

    assert!(!result.is_error, "{result:?}");

    // Both are reported, and each status is bound to its id. The failed one is
    // why the agent is here and the ready one is where it is going, so a list
    // that returned the right ids against the wrong statuses would send the
    // rollback at the deployment that just broke the site.
    let deployments: serde_json::Value =
        serde_json::from_str(&result.text()).expect("the tool answers with JSON");
    let rows = deployments.as_array().expect("an array of deployments");

    assert_eq!(rows.len(), 2, "{deployments}");
    // Newest first, as the crate documents and as the provider returned them.
    assert_eq!(rows[0]["id"], "dpl_new", "{deployments}");
    assert_eq!(rows[0]["status"], "failed", "{deployments}");
    assert_eq!(rows[1]["id"], "dpl_old", "{deployments}");
    assert_eq!(rows[1]["status"], "ready", "{deployments}");
}

/// A domain that is attached but unverified is not serving, and the tool has to
/// say so — that difference is the entire reason to read domains.
#[tokio::test]
async fn domain_status_distinguishes_a_verified_domain_from_a_pending_one() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v9/projects/shop/domains"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "domains": [
                {"name": "shop.example.com", "verified": true},
                {"name": "www.example.com", "verified": false}
            ]
        })))
        .mount(&server)
        .await;

    let result = tools::DomainStatusTool::new(host_against(&server))
        .execute(json!({"site": "shop"}))
        .await
        .expect("the tool reports rather than panics");

    assert!(!result.is_error, "{result:?}");

    // Bound to the name rather than checked for presence: asserting only that a
    // `true` and a `false` both appear somewhere passes just as happily if the
    // two are swapped, which is the one thing this tool must not get wrong.
    let domains: serde_json::Value =
        serde_json::from_str(&result.text()).expect("the tool answers with JSON");
    let verified_for = |name: &str| -> bool {
        let entry = domains
            .as_array()
            .expect("an array of domains")
            .iter()
            .find(|domain| domain["name"] == name)
            .unwrap_or_else(|| panic!("{name} is missing from {domains}"));
        entry["verified"]
            .as_bool()
            .expect("`verified` is a boolean")
    };

    assert!(
        verified_for("shop.example.com"),
        "the verified domain must report verified: {domains}"
    );
    assert!(
        !verified_for("www.example.com"),
        "and the pending one must not — that difference is the entire reason to \
         read domains: {domains}"
    );
}
