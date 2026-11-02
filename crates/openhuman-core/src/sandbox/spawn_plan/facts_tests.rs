use super::*;
use crate::sandbox::types::SandboxBackendKind;
use std::path::PathBuf;
use tinysecurity_bus::{
    ConstraintEnforcement2, ConstraintSupport2, IsolationLevel2, SandboxBackend as Backend,
    SandboxConstraint2,
};

fn policy(workspace: &str) -> SandboxPolicy {
    SandboxPolicy {
        backend: SandboxBackendKind::Local,
        workspace_root: PathBuf::from(workspace),
        state_dir: PathBuf::from("/state"),
        read_only_mounts: vec![],
        read_write_mounts: vec![],
        allow_network: false,
        env_passthrough: vec![],
        docker_overrides: None,
        mount_workspace: true,
    }
}

fn executor(id: &str) -> SandboxExecutor2 {
    SandboxExecutor2 {
        executor_id: id.into(),
        capability_id: "rev".into(),
        backend: Backend::Jail,
        available: true,
        isolation: IsolationLevel2::Kernel,
        constraints: vec![ConstraintSupport2 {
            constraint: SandboxConstraint2::Filesystem,
            enforcement: ConstraintEnforcement2::Enforced,
        }],
    }
}

fn subject() -> SubjectScope {
    SubjectScope {
        user: "local".into(),
        workspace: "/state".into(),
        agent: "agent".into(),
    }
}

fn facts(
    config: &Config,
    policy: &SandboxPolicy,
    adjust: impl FnOnce(&mut FactsInput<'_>),
) -> SandboxScope2 {
    let mut input = FactsInput {
        config,
        policy,
        source: SpawnSource::Agent,
        subject: subject(),
        saas: false,
        operator_off: false,
        executors: vec![executor("jail")],
    };
    adjust(&mut input);
    scope(&input).expect("facts")
}

#[test]
fn equal_facts_share_an_identity_and_any_difference_gets_its_own() {
    let config = Config::default();
    let base = policy("/work/action");
    let reference = facts(&config, &base, |_| {});
    assert_eq!(reference, facts(&config, &base, |_| {}));
    assert!(reference.scope_id.starts_with("sbx-"));
    assert!(reference.scope_id.len() <= 256);

    let moved = policy("/work/other");
    let networked = SandboxPolicy {
        allow_network: true,
        ..base.clone()
    };
    let mounted = SandboxPolicy {
        read_only_mounts: vec![PathBuf::from("/srv/tools")],
        ..base.clone()
    };
    let variants = [
        facts(&config, &moved, |_| {}),
        facts(&config, &networked, |_| {}),
        facts(&config, &mounted, |_| {}),
        facts(&config, &base, |i| i.saas = true),
        facts(&config, &base, |i| i.operator_off = true),
        facts(&config, &base, |i| i.source = SpawnSource::Untrusted),
        facts(&config, &base, |i| i.executors = vec![executor("other")]),
        facts(&config, &base, |i| i.subject.agent = "other".into()),
    ];
    let mut ids: Vec<_> = variants.iter().map(|v| v.scope_id.clone()).collect();
    ids.push(reference.scope_id);
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        variants.len() + 1,
        "every variant has its own scope"
    );
}

#[test]
fn deployment_origin_and_switches_follow_the_host_state() {
    let config = Config::default();
    let base = policy("/work/action");
    let local = facts(&config, &base, |_| {});
    assert_eq!(local.deployment, SandboxDeployment2::Local);
    assert_eq!(local.origin, Origin::Operator);
    assert_eq!(local.environment, SandboxSetting2::Enabled);
    assert_eq!(local.agent_mode, SandboxSetting2::Enabled);
    assert_eq!(local.configured, SandboxSetting2::Disabled);
    assert_eq!(local.tier, AccessTier::Full);

    let saas = facts(&config, &base, |i| i.saas = true);
    assert_eq!(saas.deployment, SandboxDeployment2::Saas);
    assert_eq!(saas.origin, Origin::Saas);

    let off = facts(&config, &base, |i| i.operator_off = true);
    assert_eq!(off.environment, SandboxSetting2::Disabled);
}

#[test]
fn the_operators_setting_is_declared_even_under_saas_for_the_module_to_ignore() {
    let config = Config::default();
    let base = policy("/work/action");
    let saas_off = facts(&config, &base, |i| {
        i.saas = true;
        i.operator_off = true;
    });
    assert_eq!(saas_off.deployment, SandboxDeployment2::Saas);
    assert_eq!(
        saas_off.environment,
        SandboxSetting2::Disabled,
        "the host reports the setting; refusing it is the module's job"
    );
}

#[test]
fn every_scope_asks_to_be_sandboxed_whichever_executor_the_policy_names() {
    let config = Config::default();
    for backend in [
        SandboxBackendKind::None,
        SandboxBackendKind::Local,
        SandboxBackendKind::Docker,
    ] {
        let named = SandboxPolicy {
            backend,
            ..policy("/work/action")
        };
        assert_eq!(
            facts(&config, &named, |_| {}).agent_mode,
            SandboxSetting2::Enabled
        );
    }
}

#[test]
fn the_scope_declares_every_executor_it_is_given() {
    let config = Config::default();
    let scope = facts(&config, &policy("/work/action"), |i| {
        i.executors = vec![executor("jail"), executor("docker")];
    });
    let ids: Vec<_> = scope
        .executors
        .iter()
        .map(|e| e.executor_id.as_str())
        .collect();
    assert_eq!(ids, ["jail", "docker"]);
}

#[test]
fn a_configured_backend_enables_the_configured_switch() {
    let base = policy("/work/action");
    for (backend, expected) in [
        (SandboxBackend::Auto, SandboxSetting2::Disabled),
        (SandboxBackend::None, SandboxSetting2::Disabled),
        (SandboxBackend::Landlock, SandboxSetting2::Enabled),
        (SandboxBackend::Docker, SandboxSetting2::Enabled),
    ] {
        let mut config = Config::default();
        config.sandbox.backend = backend;
        assert_eq!(facts(&config, &base, |_| {}).configured, expected);
    }
}

#[test]
fn sources_carry_the_trust_that_sets_the_floor() {
    let config = Config::default();
    let base = policy("/work/action");
    let agent = facts(&config, &base, |_| {});
    let untrusted = facts(&config, &base, |i| i.source = SpawnSource::Untrusted);
    assert_eq!(agent.sources[0].trust, SourceTrust::TrustedHost);
    assert_eq!(untrusted.sources[0].trust, SourceTrust::UntrustedSkill);
    assert_eq!(agent.sources[0].source_id, SpawnSource::Agent.source_id());
    assert_eq!(
        untrusted.sources[0].provenance,
        SpawnSource::Untrusted.provenance()
    );
}

#[test]
fn the_ceiling_is_the_workspace_and_the_mounts_the_host_resolved() {
    let mut base = policy("/work/action");
    base.read_only_mounts = vec![PathBuf::from("/opt"), PathBuf::from("/srv/tools")];
    base.read_write_mounts = vec![PathBuf::from("/home/u/.cargo/registry")];
    let ceiling = grant_ceiling(&base).expect("ceiling");
    let described: Vec<_> = ceiling.iter().map(|g| (g.path.as_str(), g.write)).collect();
    assert_eq!(
        described,
        [
            ("/home/u/.cargo/registry", true),
            ("/opt", false),
            ("/srv/tools", false),
            ("/work/action", true),
        ]
    );
    // The same roots are trusted in the owner filesystem policy.
    let scope = facts(&Config::default(), &base, |_| {});
    for (path, access) in [
        ("/work/action", PathAccess::ReadWrite),
        ("/opt", PathAccess::ReadOnly),
        ("/home/u/.cargo/registry", PathAccess::ReadWrite),
    ] {
        assert!(scope
            .paths
            .trusted_roots
            .iter()
            .any(|root| root.path == std::path::Path::new(path) && root.access == access));
    }
    assert_eq!(scope.grant_ceiling, ceiling);
}

#[test]
fn network_reach_is_none_unless_the_policy_allows_the_host_network() {
    let mut base = policy("/work/action");
    assert_eq!(network_ceiling(&base), NetworkPolicy::None);
    base.allow_network = true;
    assert_eq!(network_ceiling(&base), NetworkPolicy::Host);
}

#[test]
fn the_scope_declares_the_operators_network_allowance_to_the_module() {
    let config = Config::default();
    let mut allowed = policy("/work/action");
    allowed.allow_network = true;
    let scope = facts(&config, &allowed, |_| {});
    assert_eq!(scope.network_ceiling, NetworkPolicy::Host);
    assert!(
        scope.operator_host_network,
        "an allowed network is declared, or the module wants a per-command approval"
    );

    let offline = facts(&config, &policy("/work/action"), |_| {});
    assert_eq!(offline.network_ceiling, NetworkPolicy::None);
    assert!(
        !offline.operator_host_network,
        "a policy without network declares no allowance"
    );
}

#[test]
fn only_the_agents_own_commands_accept_a_process_level_jail() {
    assert!(SpawnSource::Agent.accepts_process_isolation());
    assert!(!SpawnSource::Untrusted.accepts_process_isolation());
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_text_cannot_be_named_to_the_module() {
    use std::os::unix::ffi::OsStrExt;
    let mut base = policy("/work/action");
    base.read_only_mounts = vec![PathBuf::from(std::ffi::OsStr::from_bytes(b"/bad/\xff"))];
    assert!(grant_ceiling(&base).is_none());
    let input = FactsInput {
        config: &Config::default(),
        policy: &base,
        source: SpawnSource::Agent,
        subject: subject(),
        saas: false,
        operator_off: false,
        executors: vec![],
    };
    assert!(scope(&input).is_none());
}

#[test]
fn a_read_only_mount_the_jail_baseline_already_covers_is_not_requested() {
    let mut base = policy("/work/action");
    base.read_only_mounts = vec![
        PathBuf::from("/usr/local"),
        PathBuf::from("/usr"),
        PathBuf::from("/lib64/python"),
        PathBuf::from("/opt"),
        PathBuf::from("/usrx/share"),
    ];
    let described: Vec<_> = grant_ceiling(&base)
        .expect("ceiling")
        .into_iter()
        .map(|g| (g.path, g.write))
        .collect();
    assert_eq!(
        described,
        [
            ("/opt".to_owned(), false),
            ("/usrx/share".to_owned(), false),
            ("/work/action".to_owned(), true),
        ],
        "only the baseline's own subtree is dropped, never a lookalike"
    );
}

#[test]
fn the_workspace_grant_is_not_blocked_by_the_path_tools_reserved_names() {
    let scope = facts(&Config::default(), &policy("/work/action"), |_| {});
    assert!(scope.paths.reserved_names.is_empty());
    // The core's internal state stays reserved.
    assert!(!scope.paths.reserved_paths.is_empty());
    assert!(!scope.paths.reserved_prefixes.is_empty());
}

#[test]
fn missing_and_remote_turn_origins_are_untrusted_spawn_sources() {
    use crate::core::runtime::turn_origin::AgentTurnOrigin;

    assert_eq!(SpawnSource::from_origin(None), SpawnSource::Untrusted);
    assert_eq!(
        SpawnSource::from_origin(Some(&AgentTurnOrigin::Unknown)),
        SpawnSource::Untrusted
    );
    assert_eq!(
        SpawnSource::from_origin(Some(&AgentTurnOrigin::ExternalChannel {
            channel: "test".into(),
            sender: None,
            sender_name: None,
            reply_target: "test".into(),
            message_id: "test".into(),
            history_key: None,
        })),
        SpawnSource::Untrusted
    );
    assert_eq!(
        SpawnSource::from_origin(Some(&AgentTurnOrigin::DirectChat)),
        SpawnSource::Agent
    );
}
