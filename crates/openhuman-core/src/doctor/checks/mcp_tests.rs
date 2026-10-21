use super::*;
use crate::doctor::contract::{DoctorOptions, FindingSeverity};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FixedStatuses(Result<Option<Vec<tinymcp_bus::ConnStatus>>, McpStatusFailure>);

#[async_trait]
impl McpStatusProbe for FixedStatuses {
    async fn statuses(
        &self,
        _config: &crate::config::Config,
    ) -> Result<Option<Vec<tinymcp_bus::ConnStatus>>, McpStatusFailure> {
        self.0.clone()
    }
}

fn server(name: &str) -> McpServerConfig {
    McpServerConfig {
        server: tinymcp_bus::McpServerConfig {
            name: name.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn config_in(root: &Path) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.join("actions");
    config.config_path = root.join("config.toml");
    config
}

fn status(server_id: &str, status: tinymcp_bus::ServerStatus) -> tinymcp_bus::ConnStatus {
    tinymcp_bus::ConnStatus {
        server_id: server_id.to_owned(),
        qualified_name: "example/server".to_owned(),
        display_name: "fixture".to_owned(),
        status,
        tool_count: 0,
        last_error: Some("private endpoint response and token=secret".to_owned()),
        auth_hint: None,
    }
}

#[tokio::test]
async fn a_failed_existing_server_is_an_error_without_its_raw_error_text() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let check = McpServerCheck::with_probe(
        &server("server-a"),
        Arc::new(FixedStatuses(Ok(Some(vec![status(
            "server-a",
            tinymcp_bus::ServerStatus::Error,
        )])))),
    );

    let finding = check
        .detect(&DoctorCtx::from_config(config_in(scratch.path())))
        .await
        .remove(0);

    assert_eq!(finding.check_id, "mcp/server-a");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding.message.contains("status: error"));
    assert!(!finding.message.contains("private endpoint"));
    assert!(!finding.message.contains("secret"));
}

#[tokio::test]
async fn an_absent_existing_client_is_skipped_without_bootstrapping_one() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let check = McpServerCheck::with_probe(&server("server-a"), Arc::new(FixedStatuses(Ok(None))));

    let finding = check
        .detect(&DoctorCtx::from_config(config_in(scratch.path())))
        .await
        .remove(0);

    assert_eq!(finding.severity, FindingSeverity::Skipped);
    assert!(finding.message.contains("already open"));
}

#[tokio::test]
async fn an_existing_runtime_status_failure_is_reported_as_an_error() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let check = McpServerCheck::with_probe(
        &server("server-a"),
        Arc::new(FixedStatuses(Err(McpStatusFailure::Failed))),
    );

    let finding = check
        .detect(&DoctorCtx::from_config(config_in(scratch.path())))
        .await
        .remove(0);

    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding.message.contains("did not return server status"));
}

#[tokio::test]
async fn disabled_mcp_component_skips_each_configured_server() {
    let workspace = tempfile::tempdir().expect("create isolated config fixture");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().join("workspace");
    config.action_dir = workspace.path().join("actions");
    config.config_path = workspace.path().join("config.toml");
    config.mcp_client.servers.push(server("server-a"));
    let mut components = crate::core::runtime::ComponentSet::all();
    components.mcp = false;
    let report = crate::doctor::runner::run(
        &DoctorCtx::from_config(config).with_components(components),
        DoctorOptions {
            only: vec!["mcp/server-a".into()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("disabled MCP component is represented as skipped");

    assert_eq!(report.checks_skipped, vec!["mcp/server-a".to_owned()]);
    assert!(report.findings.iter().any(|finding| {
        finding.check_id == "mcp/server-a" && finding.severity == FindingSeverity::Skipped
    }));
}

struct CountingProbe(Arc<AtomicUsize>);

#[async_trait]
impl McpStatusProbe for CountingProbe {
    async fn statuses(
        &self,
        _config: &crate::config::Config,
    ) -> Result<Option<Vec<tinymcp_bus::ConnStatus>>, McpStatusFailure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
}

#[tokio::test]
async fn duplicate_mcp_definitions_report_one_error_without_probing_either_entry() {
    let scratch = tempfile::tempdir().expect("create isolated config fixture");
    let calls = Arc::new(AtomicUsize::new(0));
    let servers = [
        server("server-a"),
        McpServerConfig {
            server: tinymcp_bus::McpServerConfig {
                name: "server-a".to_owned(),
                endpoint: "https://example.invalid/mcp".to_owned(),
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    let checks = checks_for_servers(&servers, Arc::new(CountingProbe(Arc::clone(&calls))));

    assert_eq!(checks.len(), 1, "one exact id represents the ambiguity");
    let finding = checks[0]
        .detect(&DoctorCtx::from_config(config_in(scratch.path())))
        .await
        .remove(0);

    assert_eq!(finding.check_id, "mcp/server-a");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding.message.contains("Multiple MCP server definitions"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn duplicate_mcp_definition_errors_are_available_at_the_static_tier() {
    let servers = [server("server-a"), server("server-a")];
    let checks = checks_for_servers(&servers, Arc::new(FixedStatuses(Ok(None))));

    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].tier(), DoctorTier::Static);
}

#[test]
fn arbitrary_server_names_produce_valid_noncolliding_ids() {
    let first =
        McpServerCheck::configured(&server("Server A/α"), Arc::new(FixedStatuses(Ok(None))));
    let second =
        McpServerCheck::configured(&server("server a/α"), Arc::new(FixedStatuses(Ok(None))));

    assert_eq!(first.id(), "mcp/__x_i_53657276657220412fceb1");
    assert_ne!(first.id(), second.id());
}
