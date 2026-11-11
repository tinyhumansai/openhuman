use super::*;

#[test]
fn enabled_policy_expands_home_relative_forbidden_paths() {
    let root = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.action_dir = root.path().join("action");
    config.workspace_dir = root.path().join("workspace");
    config.autonomy.enabled = true;
    config.autonomy.allowed_commands = vec!["pwd".into()];
    config.autonomy.forbidden_paths = vec!["~/.ssh".into(), "~/.aws".into()];

    assert_eq!(validate(&config), Ok(()));
    let translated = policy_config(&config);
    assert_eq!(
        translated.forbidden_paths,
        [
            crate::config::expand_tilde("~/.ssh"),
            crate::config::expand_tilde("~/.aws")
        ]
    );
    assert!(translated
        .forbidden_paths
        .iter()
        .all(|path| std::path::Path::new(path).is_absolute()));
}

/// TS-E2E-015 / #8575: the shipped defaults include Unix roots (`/etc`, …),
/// which have no drive on Windows. Enabling autonomy with the defaults must
/// still give the module a policy it accepts there.
#[test]
fn windows_policy_leaves_out_drive_less_unix_roots() {
    let defaults = Config::default().autonomy.forbidden_paths;
    assert!(defaults.iter().any(|path| path == "/etc"), "{defaults:?}");

    // Every default is a Unix root or a `~` path; on Windows only the `~`
    // paths remain (checked on the input: `~` expands to `/home/...` here).
    let home: Vec<String> = defaults
        .iter()
        .filter(|path| path.starts_with('~'))
        .map(|path| crate::config::expand_tilde(path))
        .collect();
    assert!(home.contains(&crate::config::expand_tilde("~/.ssh")));
    assert_eq!(forbidden_paths(&defaults, true), home);
    assert_eq!(forbidden_paths(&defaults, false).len(), defaults.len());

    let kept = [
        "C:\\Secrets",
        "\\\\server\\share",
        "//server/share",
        "relative",
    ];
    let kept: Vec<String> = kept.iter().map(|path| path.to_string()).collect();
    assert_eq!(forbidden_paths(&kept, true), kept);
}

/// The untouched defaults, enabled, validate on this OS.
#[test]
fn enabled_default_policy_validates() {
    let root = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.action_dir = root.path().join("action");
    config.workspace_dir = root.path().join("workspace");
    config.autonomy.enabled = true;

    assert_eq!(validate(&config), Ok(()));
}

#[test]
fn enabled_policy_still_refuses_relative_forbidden_paths() {
    let mut config = Config::default();
    config.autonomy.enabled = true;
    config.autonomy.allowed_commands = vec!["pwd".into()];
    config.autonomy.forbidden_paths = vec!["relative/private".into()];

    assert_eq!(
        validate(&config),
        Err("enabled policy forbidden paths must be absolute")
    );
}

#[test]
fn policy_translation_copies_effective_host_settings() {
    // Absolute on every OS: a drive-less `/private` is left out on Windows.
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private").display().to_string();
    let mut config = Config::default();
    config.autonomy.enabled = true;
    config.autonomy.allowed_commands = vec!["pwd".into()];
    config.autonomy.forbidden_paths = vec![private.clone()];

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
    assert_eq!(translated.forbidden_paths, [private]);
}

#[test]
fn first_load_and_reinit_share_one_policy_translator() {
    let _env = crate::config::test_env::EnvVarGuard::locked_unset("OPENHUMAN_KEYRING_BACKEND");
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
    assert_eq!(keyring.consent_timeout_ms, KEYRING_CONSENT_TIMEOUT_MS);
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

#[test]
fn secret_authority_bootstrap_enables_bounded_pairing_and_legacy_source_bindings() {
    let module =
        module_config_for_secret_authority(&Config::default(), Some(":1.42".into()), None).unwrap();

    let device = module.device.expect("DeviceConfig2 is enabled");
    assert_eq!(device.max_handles, 512);
    assert_eq!(device.max_bootstrap_nonces, 4096);
    let pairing = module
        .device_pairing
        .expect("DevicePairingConfig2 is enabled");
    assert_eq!(pairing.max_pairings, 256);
    let runtime = pairing.runtime_scope.unwrap();
    assert_eq!(runtime.persistence_backend_id, pairing_backend_id());
    assert_eq!(
        runtime.legacy_device_backend_id,
        Some(pairing_legacy_backend_id())
    );
}

/// A second tenant in one process must not meet a secret scope the first one
/// configured at init: the authority boots neutral and every tenant
/// registers its own scope at runtime.
#[test]
fn secret_authority_boots_without_any_tenants_secret_scope() {
    let module =
        module_config_for_secret_authority(&Config::default(), Some(":1.42".into()), None).unwrap();

    let secret = module.secret.expect("the secret authority is enabled");
    assert!(secret.registrations.is_empty(), "no static registration");
    assert!(module.secret_cipher.is_none(), "no init-owned cipher");
    let approvals = module.approvals.expect("approvals are enabled");
    assert_eq!(
        approvals.scopes.len(),
        1,
        "only the bootstrap approval scope"
    );
    assert_eq!(approvals.scopes[0].subject.workspace, "bootstrap");
    let audit = module.audit.expect("audit is enabled");
    assert_eq!(audit.scope.workspace, "bootstrap");
    assert!(
        audit.required,
        "TinySecurity binds a runtime cipher only to a required audit client"
    );
}

/// Audited permissions are appended on the init audit stream, so the identity
/// the host correlates them on must be exactly the one TinySecurity was given.
#[test]
fn audited_permissions_are_correlated_on_the_init_audit_stream() {
    let module =
        module_config_for_secret_authority(&Config::default(), Some(":1.42".into()), None).unwrap();
    let audit = module.audit.expect("audit is enabled");
    let (subject, stream_id) = init_audit_identity();
    assert_eq!(audit.scope, subject);
    assert_eq!(audit.stream_id, stream_id);
}

#[tokio::test]
async fn a_tenants_secret_registration_owns_its_cipher_on_its_own_audit_stream() {
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
    let ids = SecretRegistrationIds {
        registration_id: "1".repeat(32),
        consumer_id: "2".repeat(32),
        target_backend_id: "3".repeat(32),
        legacy_backend_id: "4".repeat(32),
    };

    let request = secret_scope_request(&scope, &ids);

    assert_eq!(request.registration.registration_id, ids.registration_id);
    assert_eq!(request.registration.approval_scope_id, scope.scope_id());
    assert_eq!(
        request.registration.target.backend_id,
        ids.target_backend_id
    );
    assert_eq!(
        request.registration.legacy[0].backend_id,
        ids.legacy_backend_id
    );
    let cipher = request.cipher.expect("a registration-owned cipher");
    assert_eq!(cipher.consumer_id, ids.consumer_id);
    assert_eq!(cipher.audit_stream_id, scope.stream_id());
    assert!(cipher.enabled);
    assert!((1..=128).contains(&cipher.slots));
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
    let translated = module_config_for_host(&config, Some(":1.42".into()), None).unwrap();

    assert_eq!(translated.trusted_host.as_deref(), Some(":1.42"));
    assert_eq!(translated.contract_version, MODULE_CONTRACT_VERSION);
    assert_eq!(translated.policy, policy_config(&config));
}

#[test]
fn module_bootstrap_rejects_an_absent_trusted_host_identity() {
    assert_eq!(
        module_config_for_host(&Config::default(), None, None),
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
fn module_contract_requires_the_exact_host_wire_version() {
    assert!(supports_contract((2, 1)));
    assert!(!supports_contract((1, 35)));
    assert!(!supports_contract((2, 0)));
    assert!(!supports_contract((2, 2)));
    assert!(!supports_contract((3, 0)));
    assert!(!supports_contract((1, 34)));
    assert!(!supports_contract((1, 27)));
}

#[test]
fn the_host_config_carries_the_judge_it_is_given() {
    let judge = super::super::judge::tests::a_judge();
    let translated = module_config_for_host(
        &Config::default(),
        Some(":1.42".into()),
        Some(judge.clone()),
    )
    .unwrap();

    assert_eq!(translated.judge, Some(judge));
}

fn keyring_workspace() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("oh-policy-keyring")
        .join("workspace")
}

fn encrypted_store_beside_host_keyring(
    workspace: &std::path::Path,
) -> tinysecurity_bus::KeyringBackend2 {
    tinysecurity_bus::KeyringBackend2::EncryptedFile {
        directory: workspace
            .join("keyring")
            .join("tinysecurity")
            .display()
            .to_string(),
    }
}

/// On a machine with no usable OS keyring the host's own secret storage runs
/// on its file backends; TinySecurity's keyring must follow it there or every
/// keyring call is denied.
#[test]
fn file_keyring_selections_use_an_encrypted_store_beside_the_host_keyring() {
    let workspace = keyring_workspace();
    for selection in ["file", "encrypted_file", " Encrypted_File "] {
        assert_eq!(
            module_keyring_backend(Some(selection), &workspace),
            Some(encrypted_store_beside_host_keyring(&workspace)),
            "{selection:?}"
        );
    }
}

#[test]
fn os_unset_and_unknown_keyring_selections_stay_on_the_os_store() {
    let workspace = keyring_workspace();
    for selection in [None, Some("os"), Some(" OS "), Some("bogus")] {
        assert_eq!(
            module_keyring_backend(selection, &workspace),
            Some(tinysecurity_bus::KeyringBackend2::Os),
            "{selection:?}"
        );
    }
}

/// `memory` keeps the host's secrets out of every durable store, so the module
/// gets no keyring at all and refuses each keyring call, rather than writing
/// them into the OS keyring the operator opted out of.
#[test]
fn memory_keyring_selection_configures_no_module_keyring() {
    assert_eq!(
        module_keyring_backend(Some("memory"), &keyring_workspace()),
        None
    );
}

/// The module rejects a relative store directory at load, which would take
/// the whole security module down; refuse only the keyring instead.
#[test]
fn file_keyring_selection_with_a_relative_workspace_configures_no_module_keyring() {
    assert_eq!(
        module_keyring_backend(Some("file"), std::path::Path::new("relative/workspace")),
        None
    );
}

#[test]
fn module_bootstrap_keyring_follows_the_host_keyring_backend_setting() {
    let _env =
        crate::config::test_env::EnvVarGuard::locked_set("OPENHUMAN_KEYRING_BACKEND", "file");
    let mut config = Config::default();
    config.workspace_dir = keyring_workspace();

    let keyring = module_config(&config)
        .crypto
        .expect("TinySecurity crypto is configured")
        .keyring
        .expect("file selection configures the module keyring");

    assert_eq!(
        keyring.backend,
        encrypted_store_beside_host_keyring(&config.workspace_dir)
    );
    assert_eq!(keyring.service, "openhuman");
    assert_eq!(keyring.consent_timeout_ms, KEYRING_CONSENT_TIMEOUT_MS);
}

#[test]
fn module_bootstrap_has_no_keyring_under_the_memory_setting() {
    let _env =
        crate::config::test_env::EnvVarGuard::locked_set("OPENHUMAN_KEYRING_BACKEND", "memory");
    let crypto = module_config(&Config::default())
        .crypto
        .expect("TinySecurity crypto stays configured");
    assert!(crypto.keyring.is_none());
}
