use super::*;
use tinysecurity_bus::{AccessTier, DenialReason, Verdict};

fn config() -> Config {
    let mut config = Config::default();
    config.action_dir = std::env::current_dir().unwrap();
    config.workspace_dir = config.action_dir.join("host-state");
    config
}

#[test]
fn initialization_does_not_treat_bootstrap_policy_as_an_agents_enabled_policy() {
    let mut config = config();
    config.security.autonomy.enabled = true;
    // `trusted_host` names the host's own bus connection, present only once
    // some other test has started the shared runtime; the policy is the claim.
    let actual = ModuleConfig {
        trusted_host: None,
        // The production password-only crypto surface is asserted separately.
        crypto: None,
        // The init-only approval, audit and sandbox surfaces carry no agent policy;
        // they are asserted separately below.
        approvals: None,
        audit: None,
        sandbox: None,
        ..module_config(&config)
    };
    assert_eq!(actual, ModuleConfig::default());
}

#[test]
fn sandbox_scopes_are_registered_at_runtime_never_at_init() {
    let configured = module_config(&config());
    let sandbox = configured.sandbox.as_ref().expect("sandbox init config");
    // The module caps init scopes at 128 and binds each to the one audit subject, while
    // the host serves an unbounded set of subjects, so none is declared at init.
    assert_eq!(*sandbox, sandbox_init_config());
    assert!(sandbox_init_config().scopes.is_empty());
    // The same value on every load and refresh: the module refuses a changed one.
    assert_eq!(module_config(&config()).sandbox, configured.sandbox);
    // The audit the runtime scopes forward on is configured beside it.
    assert!(configured.audit.is_some() && configured.approvals.is_some());
}

#[test]
fn caller_requires_both_authenticated_identity_and_call_identity() {
    assert!(VerifiedCaller::from_host(CallerContext::default()).is_err());
    let context = CallerContext {
        agent: "operator".into(),
        call_id: "host-call".into(),
        tier: AccessTier::Read,
        ..Default::default()
    };
    assert!(VerifiedCaller::from_host(context.clone()).is_ok());
    assert!(VerifiedCaller::from_host(CallerContext {
        call_id: " ".into(),
        ..context
    })
    .is_err());
}

#[test]
fn effective_policy_must_match_config_and_supply_all_required_capabilities() {
    let config = module_config(&config());
    let mut info = PolicyInfo {
        contract_version: tinysecurity_bus::CONTRACT_VERSION,
        generation: 1,
        policy: config.policy.clone(),
        capabilities: names::METHODS.iter().map(|s| (*s).into()).collect(),
        judge: None,
        callbacks: Default::default(),
        scoped_policies: 0,
    };
    assert!(validate_info(&info, &config).is_ok());
    info.policy.enabled = true;
    assert!(validate_info(&info, &config).is_err());
    info.policy = config.policy.clone();
    info.capabilities.pop();
    assert!(validate_info(&info, &config).is_err());
    info.capabilities = names::METHODS.iter().map(|s| (*s).into()).collect();
    info.contract_version = (2, 0);
    assert!(validate_info(&info, &config).is_err());
    info.contract_version = tinysecurity_bus::CONTRACT_VERSION;
    info.generation = 0;
    assert!(validate_info(&info, &config).is_err());
}

#[test]
fn allow_and_deny_decisions_require_current_generation_and_no_cache_claim() {
    for verdict in [
        Verdict::Allow,
        Verdict::Deny {
            reason: DenialReason::Floor,
        },
    ] {
        let mut decision = Decision {
            generation: 3,
            verdict,
            cacheable: false,
        };
        assert!(validate_decision(&decision, Some(3)).is_ok());
        assert!(validate_decision(&decision, Some(2)).is_err());
        assert!(validate_decision(&decision, None).is_err());
        decision.cacheable = true;
        assert!(validate_decision(&decision, Some(3)).is_err());
    }
}

#[test]
fn attested_digest_must_be_one_of_the_compiled_release_artifacts() {
    let record = super::super::ModuleRecord {
        id: MODULE_ID,
        description: "policy",
        bus_name: names::INTERFACE,
        object_path: names::OBJECT_PATH,
        version: "1.0.0",
        release_url: "https://example.invalid",
        assets: &[super::super::PlatformAsset {
            host_key: "test",
            archive: "test.tar.gz",
            sha256: "abcdef",
        }],
        load: super::super::LoadPolicy::Eager,
        source: crate::modules::types::SourceSpec {
            submodule: "tinysecurity",
            manifest: "crates/tinysecurity-module/Cargo.toml",
            package: "tinysecurity-module",
            lib_name: "tinysecurity_module",
            features: &[],
        },
    };
    assert!(pinned(&record, "ABCDEF"));
    assert!(!pinned(&record, "fedcba"));
    assert!(!pinned(&record, ""));
}

#[tokio::test]
async fn bus_name_ownership_without_artifact_attestation_is_refused() {
    use tinybus::transport::memory::MemoryBus;
    let transport = MemoryBus::new();
    tinybus::broker::Broker::new().spawn(transport.clone());
    let attacker = tinybus::Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    attacker.request_name(names::INTERFACE).await.unwrap();
    let host = tinybus::Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = host
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .unwrap();
    let record = super::super::ModuleRecord {
        id: MODULE_ID,
        description: "policy",
        bus_name: names::INTERFACE,
        object_path: names::OBJECT_PATH,
        version: "1.0.0",
        release_url: "https://example.invalid",
        assets: &[],
        load: super::super::LoadPolicy::Eager,
        source: crate::modules::types::SourceSpec {
            submodule: "tinysecurity",
            manifest: "crates/tinysecurity-module/Cargo.toml",
            package: "tinysecurity-module",
            lib_name: "tinysecurity_module",
            features: &[],
        },
    };
    assert!(proxy.is_available().await.unwrap());
    assert_eq!(
        require_attestation(&proxy, &record).await.unwrap_err(),
        "TinySecurity recipient is not attested"
    );
}

#[test]
fn relative_resolved_module_paths_are_rejected_instead_of_becoming_execution_targets() {
    use tinysecurity_bus::{PathErrorCategory, PathValidationResult};
    assert!(validate_path_response(&PathValidationResult::Allowed {
        resolved: "relative/file".into()
    })
    .is_err());
    assert!(validate_path_response(&PathValidationResult::Allowed {
        resolved: std::env::current_dir().unwrap()
    })
    .is_ok());
    assert!(validate_path_response(&PathValidationResult::Denied {
        category: PathErrorCategory::Protected
    })
    .is_ok());
    assert!(serde_json::from_value::<PathValidationResult>(
        serde_json::json!({"status":"allowed"})
    )
    .is_err());
}

#[tokio::test]
async fn disabled_modules_are_refused_before_using_cached_client_state() {
    let mut config = config();
    config.modules.enabled = false;
    assert_eq!(
        policy_info(&config).await.unwrap_err(),
        "TinySecurity modules are disabled"
    );
}

#[tokio::test]
async fn absent_module_fails_closed_instead_of_allowing() {
    use tinybus::transport::memory::MemoryBus;
    let transport = MemoryBus::new();
    tinybus::broker::Broker::new().spawn(transport.clone());
    let host = tinybus::Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = host
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .unwrap()
        .with_timeout(Duration::from_millis(200));
    // Nothing owns the bus name (module killed or never loaded): not available,
    // and an attestation request must be an error, never a permissive answer.
    assert!(!proxy.is_available().await.unwrap());
    let record = *super::super::registry::find(MODULE_ID).unwrap();
    assert!(require_attestation(&proxy, &record).await.is_err());
}

#[test]
fn production_config_enables_password_crypt_and_leaves_keyring_and_pairing_denied() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(&root).unwrap();
    let mut config = config();
    config.workspace_dir = dir.path().canonicalize().unwrap().join("state");

    let crypto = module_config(&config).crypto.expect("crypto surface");
    assert!(
        crypto.keyring.is_none(),
        "no production caller reads a module keyring, so none is configured"
    );
    assert!(
        !config.workspace_dir.join("security-keyring").exists(),
        "building the config creates no store"
    );
    assert!(crypto.pairing.is_none());
    assert!((1..=4096).contains(&crypto.operations_per_minute));
    assert!((1..=tinysecurity_bus::MAX_CRYPTO_PAYLOAD_BYTES).contains(&crypto.max_payload_bytes));
}

#[test]
fn a_load_failure_leaves_the_client_retryable_but_a_channel_fault_latches() {
    // A slow first load (download plus start) can exceed the load budget; that
    // must not poison the process-global client for every later call.
    let mut state = ClientState {
        load_failed: true,
        ..Default::default()
    };
    state.latch();
    assert!(
        !state.faulted,
        "a load that did not finish must stay retryable"
    );
    assert!(
        !state.load_failed,
        "the exemption covers exactly one failure"
    );

    // The next failure after the module answered is a real fault.
    state.latch();
    assert!(state.faulted);
}
