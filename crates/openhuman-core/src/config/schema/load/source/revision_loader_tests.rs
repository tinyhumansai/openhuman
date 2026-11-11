use super::*;
use crate::core::runtime::{context::CoreContext, ContextOverlay, DispatchCtx, DomainSet};
use serde_json::json;
use std::sync::Arc;
use tinytools::Tool;

#[tokio::test]
async fn discovered_parent_config_is_used_to_record_agent_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = memory_backend();
    let _forced = ForcedDocumentSource::new(
        scoped(storage.as_ref(), "tenant-internal-boot-agent"),
        Scope::new("tenant-internal-boot-agent").unwrap(),
        keys(18),
    );
    let seed = config_at(tmp.path(), "global-model");
    seed.save().await.unwrap();
    let base = Config::load_from_config_path(&seed.config_path, &seed.workspace_dir)
        .await
        .unwrap();
    assert!(base.mcp_client.enabled);
    assert!(!base.autonomy.enabled);
    assert!(base.memory.agent_id.is_none());

    // Normal core boot discovers its config internally, so its context has no
    // embedder snapshot. The embed runtime then derives an agent from this
    // separately loaded base config.
    let parent = CoreContext::for_test(DomainSet::full(), Some(base.workspace_dir.clone()));
    let mut agent_config = base.clone();
    agent_config.autonomy.enabled = true;
    agent_config.mcp_client.enabled = false;
    agent_config.memory.agent_id = Some("runtime-agent-memory".into());
    let child = parent.derive_with_parent_config(
        &base,
        ContextOverlay::new(
            agent_config,
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        ),
    );

    let mut external = Config::load_from_config_path(&seed.config_path, &seed.workspace_dir)
        .await
        .unwrap();
    external.default_model = Some("external-winner".into());
    external.save().await.unwrap();

    let stale = CoreContext::scope(child.clone(), async {
        CoreContext::current_embedder_config().unwrap()
    })
    .await;
    stale.save().await.unwrap_err();
    let refreshed = CoreContext::scope(child.clone(), async {
        DispatchCtx::new(child.clone()).load_config().await.unwrap()
    })
    .await;

    assert_eq!(refreshed.default_model.as_deref(), Some("external-winner"));
    assert!(refreshed.autonomy.enabled);
    assert!(!refreshed.mcp_client.enabled);
    assert_eq!(
        refreshed.memory.agent_id.as_deref(),
        Some("runtime-agent-memory")
    );
}

#[tokio::test]
async fn proxy_config_tool_loads_a_versioned_document_before_saving() {
    let _env = crate::config::test_env::lock_env_async().await;
    let previous_proxy = crate::config::runtime_proxy_config();
    let _restore = RuntimeProxyRestore(previous_proxy);
    let tmp = tempfile::tempdir().unwrap();
    let storage = memory_backend();
    let path = tmp.path().join("config.toml");
    let workspace = tmp.path().join("workspace");

    let mut seed = Config {
        config_path: path.clone(),
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        ..Config::default()
    };
    seed.proxy.enabled = true;
    seed.proxy.http_proxy = Some("http://persisted-proxy.invalid:8080".into());
    seed.save().await.unwrap();
    let _forced = ForcedDocumentSource::new(
        scoped(storage.as_ref(), "tenant-proxy-tool-revision"),
        Scope::new("tenant-proxy-tool-revision").unwrap(),
        keys(19),
    );
    let document_config = Config::load_from_config_path(&path, &seed.workspace_dir)
        .await
        .unwrap();
    document_config.save().await.unwrap();
    let _proxy_env = crate::config::test_env::EnvVarGuard::set(
        "OPENHUMAN_HTTP_PROXY",
        "http://environment-proxy.invalid:7890",
    );

    let tool = crate::tools::ProxyConfigTool::new(
        Arc::new(document_config),
        Arc::new(crate::security::SecurityPolicy {
            autonomy: crate::security::AutonomyLevel::Supervised,
            workspace_dir: tmp.path().to_path_buf(),
            ..Default::default()
        }),
    );
    let result = tool.execute(json!({ "action": "disable" })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());

    let saved_source = for_config(&path).unwrap();
    let saved = saved_source.read().await.unwrap();
    let saved = crate::config::schema::config_from_toml_str(&saved.contents).unwrap();
    assert_eq!(
        saved.proxy.http_proxy.as_deref(),
        Some("http://persisted-proxy.invalid:8080")
    );
    assert!(!saved.proxy.enabled);
}

struct RuntimeProxyRestore(crate::config::ProxyConfig);

impl Drop for RuntimeProxyRestore {
    fn drop(&mut self) {
        crate::config::set_runtime_proxy_config(self.0.clone());
    }
}

// The allowlist store lives in `channels::host`, which the gates-off build omits.
#[cfg(feature = "channels")]
#[tokio::test]
async fn channel_allowlist_store_loads_a_versioned_document_before_saving() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = crate::config::test_env::lock_env_async().await;
    let _home = crate::config::test_env::EnvVarGuard::set("HOME", tmp.path());
    let storage = memory_backend();
    let openhuman_dir = tmp.path().join(".openhuman");
    tokio::fs::create_dir_all(&openhuman_dir).await.unwrap();
    let config_path = openhuman_dir.join("config.toml");
    let workspace_dir = openhuman_dir.join("workspace");
    let mut seed = Config {
        config_path: config_path.clone(),
        workspace_dir: workspace_dir.clone(),
        action_dir: workspace_dir.clone(),
        ..Config::default()
    };
    seed.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: String::new(),
        chat_id: None,
        allowed_users: Vec::new(),
        stream_mode: crate::config::StreamMode::Off,
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    seed.save().await.unwrap();

    let _forced = ForcedDocumentSource::new(
        scoped(storage.as_ref(), "tenant-channel-allowlist-revision"),
        Scope::new("tenant-channel-allowlist-revision").unwrap(),
        keys(20),
    );
    let document_config = Config::load_from_config_path(&config_path, &workspace_dir)
        .await
        .unwrap();
    document_config.save().await.unwrap();

    crate::channels::host::ConfigAllowlistStore
        .persist_allowed_identity("telegram", "@operator")
        .await
        .unwrap();
    let saved = Config::load_from_config_path(&config_path, &workspace_dir)
        .await
        .unwrap();
    assert_eq!(
        saved
            .channels_config
            .telegram
            .as_ref()
            .unwrap()
            .allowed_users,
        ["operator"]
    );
}

#[tokio::test]
async fn recovery_restores_the_bootstrap_file_when_document_cas_conflicts() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = memory_backend();
    let mut recovered = config_at(tmp.path(), "recovered-config");
    recovered.secrets.encrypt = false;
    recovered.storage.url = Some("sqlite:/bootstrap-from-recovered-file".into());
    recovered.save().await.unwrap();
    let corrupted = recovered.config_path.with_extension("toml.corrupted");
    std::fs::rename(&recovered.config_path, &corrupted).unwrap();

    let _forced = ForcedDocumentSource::new(
        scoped(storage.as_ref(), "tenant-recovery-cas"),
        Scope::new("tenant-recovery-cas").unwrap(),
        keys(21),
    );
    let source = for_config(&recovered.config_path).unwrap();
    let external = config_at(tmp.path(), "concurrent-document-writer");
    external.save().await.unwrap();

    let error = super::super::super::super::branches::save_recovered(&recovered, source.as_ref())
        .await
        .unwrap_err();
    assert!(
        error.chain().any(|cause| {
            cause
                .downcast_ref::<tinystoragedrivers::StorageError>()
                .is_some_and(|error| error.kind() == tinystoragedrivers::ErrorKind::Conflict)
        }),
        "document conflict should remain the returned error: {error:#}"
    );
    let restored = tokio::fs::read_to_string(&recovered.config_path)
        .await
        .unwrap();
    let restored: toml::Value = toml::from_str(&restored).unwrap();
    assert_eq!(
        restored["storage"]["url"].as_str().unwrap(),
        "sqlite:/bootstrap-from-recovered-file"
    );
}
