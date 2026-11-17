use super::*;

#[test]
fn loads_an_operator_file_with_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("operator.toml");
    std::fs::write(&path, "root = \"/srv/openhuman\"\n").unwrap();
    let config = SaasConfig::load(&path).unwrap();
    assert_eq!(config, SaasConfig::new("/srv/openhuman"));
    assert_eq!(config.sandbox.network, "none");
    assert_eq!(
        config.service_token_path(),
        PathBuf::from("/srv/openhuman/service.token")
    );
}

#[test]
fn reads_every_operator_setting() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("operator.toml");
    std::fs::write(
        &path,
        r#"
root = "/srv/oh"
service_token_file = "/run/secrets/gateway"
tool_allowlist = ["host_shell"]
rpc_allowlist_extra = ["threads.list"]
max_profiles_open = 8
profile_ids = "hashed"
idle_evict_secs = 60
shared_backend_api_key = true
custom_definitions = true
require_user_signature = false

[sandbox]
image = "registry.example/sandbox:1"
network = "egress"
memory_limit_mb = 256
cpu_limit = 0.5
"#,
    )
    .unwrap();
    let config = SaasConfig::load(&path).unwrap();
    assert_eq!(
        config.service_token_path(),
        PathBuf::from("/run/secrets/gateway")
    );
    assert_eq!(config.tool_allowlist, vec!["host_shell".to_string()]);
    assert_eq!(config.sandbox.image, "registry.example/sandbox:1");
    assert_eq!(config.sandbox.network, "egress");
    assert_eq!(config.sandbox.memory_limit_mb, 256);
    assert_eq!(config.sandbox.cpu_limit, 0.5);
    assert_eq!(config.rpc_allowlist_extra, vec!["threads.list".to_string()]);
    assert_eq!(config.max_profiles_open, 8);
    assert_eq!(config.profile_ids, crate::profiles::ProfileIdMode::Hashed);
    assert_eq!(config.idle_evict_secs, 60);
    assert!(config.shared_backend_api_key && config.custom_definitions);
    assert!(!config.require_user_signature);
}

#[test]
fn unknown_keys_are_refused() {
    // A misspelt safety setting must not be silently ignored.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("operator.toml");
    std::fs::write(
        &path,
        "root = \"/srv/oh\"\ntool_allow_list = [\"coding\"]\n",
    )
    .unwrap();
    let err = SaasConfig::load(&path).unwrap_err().to_string();
    assert!(err.contains("tool_allow_list"), "{err}");
}

#[test]
fn a_missing_file_names_the_path() {
    let err = SaasConfig::load(Path::new("/nope/operator.toml"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("/nope/operator.toml"), "{err}");
}

#[test]
fn the_operator_plane_lives_under_the_root() {
    let config = SaasConfig::new("/srv/oh");
    let operator = config.operator_config();
    for path in [
        &operator.workspace_dir,
        &operator.config_path,
        &operator.action_dir,
    ] {
        assert!(path.starts_with("/srv/oh/operator"), "{}", path.display());
    }
}

#[test]
fn the_saas_presets_are_closed() {
    assert_eq!(
        DomainSet::saas(),
        DomainSet {
            operator: true,
            // Only the reviewed Agent controls pass profiles::surface;
            // the actual dispatch regression keeps other Agent methods denied.
            agent: true,
            threads: true,
            channels: true,
            memory: true,
            ..DomainSet::none()
        },
        "the operator plane plus the user families isolated so far"
    );
    let services = ServiceSet::saas();
    assert!(services.rpc_http);
    assert_eq!(
        ServiceSet {
            rpc_http: false,
            ..services
        },
        ServiceSet::none()
    );
}

#[test]
fn boot_refuses_an_unseeded_definition_registry() {
    let err = verify_builtin_definitions(None).unwrap_err().to_string();
    assert!(err.contains("not seeded"), "{err}");
}

#[test]
fn boot_accepts_a_builtins_only_registry() {
    let registry = crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    verify_builtin_definitions(Some(&registry)).unwrap();
}

#[test]
fn boot_refuses_a_registry_with_workspace_definitions() {
    use crate::agent_registry::host_definition::AgentDefinitionRegistry;
    use tinyagents_definition::DefinitionSource;
    let mut custom = AgentDefinitionRegistry::builtins_only()
        .get("orchestrator")
        .cloned()
        .expect("built-in orchestrator");
    custom.source = DefinitionSource::File("/ws/agents/orchestrator.toml".into());
    let registry = AgentDefinitionRegistry::builtins_only().with_definitions([custom]);
    let err = verify_builtin_definitions(Some(&registry))
        .unwrap_err()
        .to_string();
    assert!(err.contains("built-ins only"), "{err}");
}

#[test]
fn cluster_settings_default_and_resolve() {
    let mut config = SaasConfig::new("/srv/oh");
    assert_eq!(config.lease_ttl_secs, 30);
    assert_eq!(config.lease_ttl(), std::time::Duration::from_secs(30));
    assert_eq!(config.operator_dir(), PathBuf::from("/srv/oh/operator"));
    assert!(!config.is_clustered());
    assert_eq!(config.storage_url_with(None), None);

    config.storage_url = Some("sqlite:/srv/oh/a.db".into());
    assert_eq!(
        config.storage_url_with(None).as_deref(),
        Some("sqlite:/srv/oh/a.db")
    );
    assert_eq!(
        config.storage_url_with(Some("sqlite:/env.db")).as_deref(),
        Some("sqlite:/env.db"),
        "OPENHUMAN_STORAGE_URL wins"
    );
    assert_eq!(
        config.storage_url_with(Some("  ")).as_deref(),
        Some("sqlite:/srv/oh/a.db"),
        "a blank environment value is unset"
    );

    assert_eq!(config.resolve_node_id(Some("node-env")), "node-env");
    let mut fresh = SaasConfig::new("/srv/oh");
    let random = fresh.resolve_node_id(None).to_owned();
    assert!(random.starts_with("node-"), "{random}");
    assert_eq!(
        fresh.resolve_node_id(Some("ignored")),
        random,
        "resolved once"
    );
    let mut configured = SaasConfig::new("/srv/oh");
    configured.node_id = Some("node-file".into());
    assert_eq!(configured.resolve_node_id(Some("node-env")), "node-file");

    config.operator_dir = Some("/srv/oh/operators/a".into());
    assert_eq!(config.operator_dir(), PathBuf::from("/srv/oh/operators/a"));
    config.advertise_url = Some("http://a:7788".into());
    assert!(config.is_clustered());
}

#[test]
fn reads_the_cluster_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("operator.toml");
    std::fs::write(
        &path,
        r#"
root = "/srv/oh"
storage_url = "sqlite:/srv/oh/shared.db"
node_id = "node-a"
advertise_url = "http://10.0.0.1:7788"
lease_ttl_secs = 5
operator_dir = "/srv/oh/operators/node-a"
"#,
    )
    .unwrap();
    let config = SaasConfig::load(&path).unwrap();
    assert_eq!(config.node_id.as_deref(), Some("node-a"));
    assert_eq!(config.lease_ttl_secs, 5);
    assert!(config.is_clustered());
    assert_eq!(
        config.operator_config().workspace_dir,
        PathBuf::from("/srv/oh/operators/node-a/workspace")
    );
}

#[test]
fn the_pre_rename_max_agents_open_key_still_sets_the_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("operator.toml");
    std::fs::write(&path, "root = \"/srv/openhuman\"\nmax_agents_open = 7\n").unwrap();
    assert_eq!(SaasConfig::load(&path).unwrap().max_profiles_open, 7);
}

#[test]
fn reads_the_operators_cron_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("operator.toml");
    std::fs::write(
        &path,
        "root = \"/srv/oh\"\n[cron]\nagent_jobs_only = true\nmax_active_jobs = 5\n",
    )
    .unwrap();
    let cron = SaasConfig::load(&path).unwrap().cron;
    assert!(cron.agent_jobs_only);
    assert_eq!((cron.min_interval_secs, cron.max_active_jobs), (300, 5));
}

#[test]
fn debug_output_redacts_the_storage_url() {
    let mut config = SaasConfig::new("/srv/oh");
    config.storage_url = Some("mongodb://admin:hunter2@db.internal/oh".into());
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("hunter2") && !shown.contains("db.internal"),
        "{shown}"
    );
    assert!(shown.contains("<redacted>"));
    assert!(!format!("{:?}", SaasConfig::new("/srv/oh")).contains("<redacted>"));
}

#[test]
fn reads_the_tool_iteration_cap() {
    let config: SaasConfig =
        toml::from_str("root = \"/srv/oh\"\nmax_tool_iterations = 4\n").unwrap();
    assert_eq!(config.max_tool_iterations, Some(4));
    assert_eq!(SaasConfig::new("/srv/oh").max_tool_iterations, None);
}
