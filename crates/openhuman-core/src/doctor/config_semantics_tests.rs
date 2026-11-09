use crate::config::{test_env::EnvVarGuard, Config};
use crate::doctor::contract::{DoctorCtx, DoctorOptions, Finding, FindingSeverity};
use crate::doctor::runner;

async fn run_check(config: Config, check_id: &str) -> Vec<Finding> {
    let report = runner::run(
        &DoctorCtx::from_config(config),
        DoctorOptions {
            only: vec![check_id.to_owned()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("built-in doctor checks should be registered");
    report
        .findings
        .into_iter()
        .filter(|finding| finding.check_id == check_id)
        .collect()
}

fn finding(findings: &[Finding], severity: FindingSeverity) -> &Finding {
    findings
        .iter()
        .find(|finding| finding.severity == severity)
        .expect("expected finding severity")
}

fn workspace_entries(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut entries = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).expect("read workspace snapshot") {
            let path = entry.expect("workspace entry").path();
            if path.is_dir() {
                pending.push(path.clone());
            }
            entries.push(path.strip_prefix(root).unwrap().to_owned());
        }
    }
    entries.sort();
    entries
}

#[tokio::test]
async fn disabled_memory_component_warns_when_an_engine_is_configured() {
    let mut config = Config::default();
    config.components.memory = false;
    config.memory.engine = "cortexdb".into();

    let findings = run_check(config, "components/referenced-but-disabled").await;

    let warning = finding(&findings, FindingSeverity::Warn);
    assert_eq!(warning.config_path.as_deref(), Some("memory.engine"));
    assert!(!warning.message.contains("cortexdb"));
}

#[tokio::test]
async fn disabled_mcp_component_warns_when_servers_are_configured() {
    let mut config = Config::default();
    config.components.mcp = false;
    let mut server = crate::config::McpServerConfig::default();
    server.name = "example".into();
    server.endpoint = "https://mcp.example.test/rpc".into();
    config.mcp_client.servers.push(server);

    let findings = run_check(config, "components/referenced-but-disabled").await;

    assert_eq!(
        finding(&findings, FindingSeverity::Warn)
            .config_path
            .as_deref(),
        Some("mcp_client.servers")
    );
}

#[tokio::test]
async fn disabled_channels_component_warns_when_a_provider_is_configured() {
    let mut config = Config::default();
    config.components.channels = false;
    config.channels_config.slack = Some(crate::config::SlackConfig {
        bot_token: "fixture-bot-token".into(),
        app_token: None,
        channel_id: None,
        allowed_users: Vec::new(),
    });

    let findings = run_check(config, "components/referenced-but-disabled").await;

    assert_eq!(
        finding(&findings, FindingSeverity::Warn)
            .config_path
            .as_deref(),
        Some("channels_config")
    );
}

#[tokio::test]
async fn disabled_voice_component_warns_when_a_provider_is_configured() {
    let mut config = Config::default();
    config.components.voice = false;
    let mut provider = crate::config::schema::voice_providers::VoiceProviderCreds::default();
    provider.slug = "test-provider".into();
    provider.endpoint = "https://voice.example.test".into();
    config.voice_providers.push(provider);

    let findings = run_check(config, "components/referenced-but-disabled").await;

    assert_eq!(
        finding(&findings, FindingSeverity::Warn)
            .config_path
            .as_deref(),
        Some("voice_providers")
    );
}

#[tokio::test]
async fn missing_storage_driver_is_reported_without_opening_the_backend() {
    let _environment = EnvVarGuard::locked_unset_async(crate::storage::STORAGE_URL_VAR).await;
    let mut config = Config::default();
    config.storage.url = Some("mongodb://app:private@db.internal/openhuman".into());

    let findings = run_check(config, "storage/url").await;

    #[cfg(not(feature = "storage-mongodb"))]
    {
        let error = finding(&findings, FindingSeverity::Error);
        assert!(error.message.contains("storage-mongodb"));
        assert!(!error.message.contains("private"));
        assert!(!error.message.contains("db.internal"));
    }
    #[cfg(feature = "storage-mongodb")]
    assert!(findings
        .iter()
        .any(|finding| finding.severity == FindingSeverity::Pass));
}

#[tokio::test]
async fn malformed_storage_url_is_reported_without_echoing_it() {
    let _environment = EnvVarGuard::locked_unset_async(crate::storage::STORAGE_URL_VAR).await;
    let mut config = Config::default();
    config.storage.url = Some("custom://secret-value".into());

    let findings = run_check(config, "storage/url").await;

    let error = finding(&findings, FindingSeverity::Error);
    assert!(!error.message.contains("secret-value"));
}

// This fixture relies on POSIX mode bits; Windows readonly attributes do not revoke write ACLs.
#[cfg(unix)]
#[tokio::test]
async fn action_directory_without_write_access_is_an_error() {
    let dir = tempfile::tempdir().expect("temporary action directory");
    let mut config = Config::default();
    config.action_dir = dir.path().to_owned();
    let original_permissions = std::fs::metadata(dir.path()).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(dir.path(), permissions).unwrap();

    let findings = run_check(config, "workspace/action-dir").await;
    std::fs::set_permissions(dir.path(), original_permissions).unwrap();

    assert_eq!(
        finding(&findings, FindingSeverity::Error)
            .config_path
            .as_deref(),
        Some("action_dir")
    );
    #[cfg(unix)]
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn missing_action_directory_is_reported_without_creating_it() {
    let dir = tempfile::tempdir().expect("temporary parent directory");
    let action_dir = dir.path().join("missing");
    let mut config = Config::default();
    config.action_dir = action_dir.clone();

    let findings = run_check(config, "workspace/action-dir").await;

    assert_eq!(
        finding(&findings, FindingSeverity::Error)
            .config_path
            .as_deref(),
        Some("action_dir")
    );
    assert!(!action_dir.exists());
}

#[tokio::test]
async fn both_master_key_environment_sources_are_reported_without_initializing_keys() {
    use crate::security::secret_storage::encrypted_file_backend::{
        MASTER_KEY_ENV, MASTER_KEY_FILE_ENV,
    };

    let _environment = EnvVarGuard::locked_unset_many_async(&[MASTER_KEY_ENV, MASTER_KEY_FILE_ENV])
        .await
        .with(MASTER_KEY_ENV, "do-not-read-inline");
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let supplied_key_file = workspace.path().join("nested/operator.key");
    let _environment = _environment.with(MASTER_KEY_FILE_ENV, &supplied_key_file);
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_owned();
    let entries_before = workspace_entries(workspace.path());

    let findings = run_check(config, "secrets/master-key-env").await;

    let error = finding(&findings, FindingSeverity::Error);
    assert!(!error.message.contains("do-not-read-inline"));
    assert!(!error
        .message
        .contains(&supplied_key_file.display().to_string()));
    assert_eq!(workspace_entries(workspace.path()), entries_before);
    assert!(!supplied_key_file.exists());
    assert!(!workspace.path().join("keyring/master.key").exists());
}

#[tokio::test]
async fn invalid_module_override_is_reported_without_echoing_its_path() {
    use crate::config::schema::ModuleOverride;

    let mut config = Config::default();
    config.modules.overrides.push(ModuleOverride {
        id: "tinydocs".into(),
        path: Some("/private/libmodule.so".into()),
        source: Some(crate::config::schema::ModuleSourceMode::Source),
    });

    let findings = run_check(config, "modules/override-invalid").await;

    let warning = finding(&findings, FindingSeverity::Warn);
    assert!(!warning.message.contains("/private/libmodule.so"));
}

#[tokio::test]
async fn source_fallback_is_reported_without_loading_modules() {
    use crate::config::schema::ModuleSourceMode;

    let source_dir = tempfile::tempdir().expect("empty module source directory");
    let _environment = EnvVarGuard::locked_unset_many_async(&[
        "OPENHUMAN_MODULE_SOURCE",
        "OPENHUMAN_MODULE_SOURCE_DIR",
    ])
    .await
    .with("OPENHUMAN_MODULE_SOURCE_DIR", source_dir.path());
    let mut config = Config::default();
    config.modules.source = ModuleSourceMode::Source;

    let findings = run_check(config, "modules/source-fallback").await;

    assert!(findings.iter().any(|finding| {
        finding.severity == FindingSeverity::Warn
            && (finding.message.contains("no source-built artifact")
                || finding.message.contains("source fallback"))
    }));
}

#[tokio::test]
async fn invalid_proxy_settings_are_reported_without_echoing_credentials() {
    let mut config = Config::default();
    config.proxy.http_proxy = Some("http://user:private@[invalid".into());

    let findings = run_check(config, "network/proxy").await;

    let error = finding(&findings, FindingSeverity::Error);
    assert!(!error.message.contains("private"));
}

#[tokio::test]
async fn invalid_memory_root_is_reported_without_echoing_the_value() {
    let mut config = Config::default();
    config.memory.root = Some("private-invalid-root".into());

    let findings = run_check(config, "memory/root").await;

    let error = finding(&findings, FindingSeverity::Error);
    assert!(!error.message.contains("private-invalid-root"));
}
