use super::*;

#[test]
fn policy_translation_copies_effective_host_settings() {
    let mut config = Config::default();
    config.autonomy.enabled = true;
    config.autonomy.allowed_commands = vec!["pwd".into()];
    config.autonomy.forbidden_paths = vec!["/private".into()];

    let translated = policy_config(&config);
    assert!(translated.enabled);
    assert_eq!(
        translated.action_dir,
        config.action_dir.display().to_string()
    );
    assert_eq!(
        translated.workspace_dir,
        config.workspace_dir.display().to_string()
    );
    assert_eq!(translated.allowed_commands, ["pwd"]);
    assert_eq!(translated.forbidden_paths, ["/private"]);
}

#[test]
fn first_load_and_reinit_share_one_policy_translator() {
    let config = Config::default();
    let module = module_config(&config);
    assert_eq!(module.policy, policy_config(&config));
    assert_eq!(
        module.sandbox,
        Some(tinysecurity_bus::SandboxConfig2 { scopes: vec![] })
    );
    let crypto = module.crypto.expect("TinySecurity keyring is configured");
    let keyring = crypto
        .keyring
        .expect("profile keyring backend is configured");
    assert_eq!(keyring.service, "openhuman");
    assert_eq!(keyring.backend, tinysecurity_bus::KeyringBackend2::Os);
    assert_eq!(keyring.consent_timeout_ms, 120_000);
}

#[test]
fn module_bootstrap_configures_the_host_served_audit_stream_from_config() {
    let mut config = Config::default();
    config.audit.enabled = true;
    config.audit.max_size_mb = 7;

    let audit = module_config(&config).audit.expect("audit sink is enabled");

    assert!(audit.native_path.is_none(), "OpenHuman serves AuditSink2");
    assert!(
        !audit.required,
        "optional policy checks have no audit correlation"
    );
    assert_eq!(audit.max_bytes, 7 * 1024 * 1024);
    assert_eq!(audit.journal_max_bytes, 7 * 1024 * 1024);
    assert_eq!(audit.journal_max_events, 10_000);
}

#[tokio::test]
async fn secret_scope_bootstrap_enables_bounded_pairing_and_legacy_source_bindings() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = temp.path().to_path_buf();
    let subject = tinysecurity_bus::SubjectScope {
        user: "user-1".into(),
        workspace: temp.path().display().to_string(),
        agent: "config-secrets".into(),
    };
    let scope = crate::security::approval::native::prepare_secret_scope(&config, subject)
        .await
        .unwrap();

    let module = module_config_for_secret_scope(
        &config,
        Some(":1.42".into()),
        &scope,
        "1".repeat(32),
        "2".repeat(32),
        "3".repeat(32),
        "4".repeat(32),
        "5".repeat(32),
    )
    .unwrap();

    let device = module.device.expect("DeviceConfig2 is enabled");
    assert_eq!(device.max_handles, 512);
    assert_eq!(device.max_bootstrap_nonces, 4096);
    let pairing = module
        .device_pairing
        .expect("DevicePairingConfig2 is enabled");
    assert_eq!(pairing.max_pairings, 256);
    let runtime = pairing.runtime_scope.unwrap();
    assert_eq!(runtime.persistence_backend_id, "5".repeat(32));
    assert!(runtime.legacy_device_backend_id.is_some());
}

#[test]
fn sandbox_planning_keeps_its_audit_sink_when_optional_audit_is_disabled() {
    let mut config = Config::default();
    config.audit.enabled = false;

    assert!(module_config(&config).audit.is_some());
}

#[test]
fn module_bootstrap_binds_the_trusted_host_identity() {
    let config = Config::default();
    let translated = module_config_for_host(&config, Some(":1.42".into())).unwrap();

    assert_eq!(translated.trusted_host.as_deref(), Some(":1.42"));
    assert_eq!(translated.contract_version, MODULE_CONTRACT_VERSION);
    assert_eq!(translated.policy, policy_config(&config));
}

#[test]
fn module_bootstrap_rejects_an_absent_trusted_host_identity() {
    assert_eq!(
        module_config_for_host(&Config::default(), None),
        Err("module host identity unavailable")
    );
}

#[test]
fn disabled_policy_clears_inert_host_defaults() {
    let mut config = Config::default();
    config.autonomy.allowed_commands = vec!["git status".into()];
    config.autonomy.forbidden_paths = vec!["relative/path".into()];

    let translated = policy_config(&config);
    assert!(!translated.enabled);
    assert!(translated.allowed_commands.is_empty());
    assert!(translated.forbidden_paths.is_empty());
    assert!(validate(&config).is_ok());
}

#[test]
fn enabled_policy_rejects_commands_the_module_cannot_represent() {
    let mut config = Config::default();
    config.autonomy.enabled = true;
    config.autonomy.allowed_commands = vec!["git status".into()];

    assert_eq!(
        validate(&config),
        Err("enabled policy allows only argument-free command names")
    );
}

#[test]
fn module_contract_requires_the_exact_published_wire_version() {
    assert!(supports_contract((1, 33)));
    assert!(!supports_contract((1, 28)));
    assert!(!supports_contract((1, 29)));
    assert!(!supports_contract((1, 32)));
    assert!(!supports_contract((2, 0)));
    assert!(!supports_contract((1, 27)));
}
