use super::*;

/// A deployment that passes every check: a private root outside any home, a
/// valid token, the SaaS presets, and a clean environment.
struct Fixture {
    _tmp: tempfile::TempDir,
    config: SaasConfig,
    token: ServiceToken,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("saas");
    std::fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    Fixture {
        config: SaasConfig::new(root),
        token: ServiceToken::Valid("x".repeat(MIN_SERVICE_TOKEN_LEN)),
        _tmp: tmp,
    }
}

fn inputs<'a>(f: &'a Fixture, env: &'a [(String, String)]) -> BootInputs<'a> {
    BootInputs {
        host_kind: HostKind::Saas,
        services: ServiceSet::saas(),
        domains: DomainSet::saas(),
        config: &f.config,
        token: &f.token,
        env,
        home: Some(PathBuf::from("/home/nobody")),
        sandbox_available: false,
    }
}

fn violations(inputs: &BootInputs<'_>) -> Vec<Violation> {
    check(inputs)
        .err()
        .map(|e| e.violations)
        .unwrap_or_default()
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn a_clean_deployment_boots() {
    let f = fixture();
    assert_eq!(check(&inputs(&f, &[])), Ok(()));
}

#[test]
fn only_the_saas_host_kind_serves_saas() {
    let f = fixture();
    for kind in [
        HostKind::TauriShell,
        HostKind::Cli,
        HostKind::Docker,
        HostKind::Library,
    ] {
        let mut i = inputs(&f, &[]);
        i.host_kind = kind;
        assert_eq!(violations(&i), vec![Violation::HostKind(kind)]);
    }
}

#[test]
fn background_services_beyond_the_preset_are_refused() {
    let f = fixture();
    let mut i = inputs(&f, &[]);
    i.services = ServiceSet::desktop();
    let found = violations(&i);
    for name in ["socketio", "cron", "channels", "login_gated", "mcp_boot"] {
        assert!(
            found.contains(&Violation::Service(name)),
            "{name}: {found:?}"
        );
    }
    assert!(!found.contains(&Violation::Service("rpc_http")));
}

#[test]
fn domain_families_are_refused_until_they_are_isolated() {
    let f = fixture();
    let mut i = inputs(&f, &[]);
    i.domains = DomainSet::harness();
    let found = violations(&i);
    for name in ["config", "security"] {
        assert!(
            found.contains(&Violation::Domain(name)),
            "{name}: {found:?}"
        );
    }
    for isolated in ["agent", "threads", "memory"] {
        assert!(
            !found.contains(&Violation::Domain(isolated)),
            "{isolated} is isolated per user: {found:?}"
        );
    }
}

#[test]
fn single_user_environment_is_refused() {
    let f = fixture();
    let vars = env(&[
        ("OPENHUMAN_WORKSPACE", "/home/u/.openhuman/users/u1"),
        ("OPENHUMAN_DEV_CONNECT", "1"),
        ("OPENHUMAN_BACKEND_SESSION_TOKEN", "jwt"),
        ("OPENHUMAN_CORE_TOKEN", "t"),
        ("OPENHUMAN_BACKEND_API_KEY", "k"),
    ]);
    let found = violations(&inputs(&f, &vars));
    assert_eq!(found.len(), 5, "{found:?}");
    assert!(found.iter().all(|v| matches!(v, Violation::Env { .. })));
}

#[test]
fn empty_values_do_not_count_as_set() {
    let f = fixture();
    let vars = env(&[("OPENHUMAN_WORKSPACE", "  ")]);
    assert_eq!(check(&inputs(&f, &vars)), Ok(()));
}

#[test]
fn protections_may_be_switched_on_but_not_off() {
    let f = fixture();
    let on = env(&[
        ("OPENHUMAN_APPROVAL_GATE", "1"),
        ("OPENHUMAN_SANDBOX", "on"),
    ]);
    assert_eq!(check(&inputs(&f, &on)), Ok(()));
    let off = env(&[
        ("OPENHUMAN_APPROVAL_GATE", "0"),
        ("OPENHUMAN_SANDBOX", "off"),
    ]);
    assert_eq!(violations(&inputs(&f, &off)).len(), 2);
}

#[test]
fn a_shared_backend_key_needs_the_operator_opt_in() {
    let mut f = fixture();
    let vars = env(&[(SHARED_API_KEY_ENV, "key")]);
    assert_eq!(violations(&inputs(&f, &vars)).len(), 1);
    f.config.shared_backend_api_key = true;
    assert_eq!(check(&inputs(&f, &vars)), Ok(()));
}

#[test]
fn unknown_tool_groups_and_rpc_extras_are_refused() {
    let mut f = fixture();
    f.config.tool_allowlist = vec!["coding".into()];
    f.config.rpc_allowlist_extra = vec!["config.get_config".into()];
    let found = violations(&inputs(&f, &[]));
    assert!(found.contains(&Violation::ToolAllowlist(vec!["coding".into()])));
    assert!(found.contains(&Violation::RpcAllowlist(vec!["config.get_config".into()])));
}

#[test]
fn known_tool_groups_are_accepted() {
    let mut f = fixture();
    f.config.tool_allowlist = vec!["host_files".into()];
    assert_eq!(check(&inputs(&f, &[])), Ok(()));
}

#[test]
fn host_shell_needs_a_working_sandbox() {
    let mut f = fixture();
    f.config.tool_allowlist = vec!["host_shell".into()];
    let found = violations(&inputs(&f, &[]));
    assert!(
        matches!(&found[..], [Violation::Sandbox(why)] if why.contains("Docker")),
        "{found:?}"
    );

    let mut i = inputs(&f, &[]);
    i.sandbox_available = true;
    assert_eq!(check(&i), Ok(()));
}

#[test]
fn a_sandbox_on_the_host_network_is_refused() {
    let mut f = fixture();
    f.config.tool_allowlist = vec!["host_shell".into()];
    f.config.sandbox.network = "host".into();
    f.config.sandbox.memory_limit_mb = 0;
    let mut i = inputs(&f, &[]);
    i.sandbox_available = true;
    let found = violations(&i);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|v| matches!(v, Violation::Sandbox(_))));
}

#[test]
fn the_sandbox_is_not_checked_without_host_shell() {
    let mut f = fixture();
    f.config.sandbox.network = "host".into();
    assert_eq!(check(&inputs(&f, &[])), Ok(()));
}

#[test]
fn a_bad_root_is_refused() {
    let mut f = fixture();
    f.config.root = PathBuf::from("relative/root");
    assert!(matches!(
        violations(&inputs(&f, &[]))[..],
        [Violation::Root(_)]
    ));

    f.config.root = PathBuf::from("/definitely/not/here/openhuman-saas");
    assert!(matches!(
        violations(&inputs(&f, &[]))[..],
        [Violation::Root(_)]
    ));
}

#[test]
fn a_root_inside_the_desktop_install_is_refused() {
    let f = fixture();
    let mut i = inputs(&f, &[]);
    // Treat the fixture's parent as the home directory and nest the root in
    // its `.openhuman`.
    let home = f.config.root.parent().unwrap().to_path_buf();
    let nested = home.join(".openhuman").join("saas");
    std::fs::create_dir_all(&nested).unwrap();
    let config = SaasConfig::new(nested);
    i.config = &config;
    i.home = Some(home);
    let found = violations(&i);
    assert!(
        matches!(&found[..], [Violation::Root(why)] if why.contains(".openhuman")),
        "{found:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_world_writable_root_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    std::fs::set_permissions(&f.config.root, std::fs::Permissions::from_mode(0o777)).unwrap();
    let found = violations(&inputs(&f, &[]));
    assert!(
        matches!(&found[..], [Violation::Root(why)] if why.contains("world-writable")),
        "{found:?}"
    );
}

#[test]
fn an_invalid_token_is_refused() {
    let mut f = fixture();
    f.token = ServiceToken::Invalid("missing".into());
    assert_eq!(
        violations(&inputs(&f, &[])),
        vec![Violation::ServiceToken("missing".into())]
    );
}

#[test]
fn every_violation_is_reported_at_once() {
    let mut f = fixture();
    f.token = ServiceToken::Invalid("missing".into());
    f.config.tool_allowlist = vec!["coding".into()];
    let vars = env(&[("OPENHUMAN_WORKSPACE", "/w")]);
    let mut i = inputs(&f, &vars);
    i.host_kind = HostKind::Cli;
    let err = check(&i).unwrap_err();
    assert_eq!(err.violations.len(), 4, "{err}");
    let text = err.to_string();
    assert!(text.contains("4 problem(s)"), "{text}");
}

#[test]
fn service_token_file_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("service.token");

    assert!(matches!(
        ServiceToken::read(&path),
        ServiceToken::Invalid(_)
    ));

    std::fs::write(&path, "short\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        matches!(ServiceToken::read(&path), ServiceToken::Invalid(why) if why.contains("at least"))
    );

    let bearer = "b".repeat(MIN_SERVICE_TOKEN_LEN + 4);
    std::fs::write(&path, format!("{bearer}\n")).unwrap();
    assert_eq!(
        ServiceToken::read(&path),
        ServiceToken::Valid(bearer.clone())
    );

    // A token no client could send as a bearer header is refused.
    for bad in [
        format!("{bearer}\n{bearer}\n"),
        format!("{bearer} {bearer}"),
        format!("{bearer}\u{7}"),
        format!("{bearer}é"),
    ] {
        std::fs::write(&path, bad).unwrap();
        assert!(
            matches!(ServiceToken::read(&path), ServiceToken::Invalid(why) if why.contains("visible ASCII"))
        );
    }
    std::fs::write(&path, format!("{bearer}\n")).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            matches!(ServiceToken::read(&path), ServiceToken::Invalid(why) if why.contains("readable by others"))
        );
    }
}

#[test]
fn sandbox_none_counts_as_switched_off() {
    let f = fixture();
    let vars = env(&[("OPENHUMAN_SANDBOX", "none")]);
    assert!(matches!(
        violations(&inputs(&f, &vars))[..],
        [Violation::Env { .. }]
    ));
}

#[cfg(unix)]
#[test]
fn a_root_aliasing_the_desktop_install_is_refused() {
    let f = fixture();
    let home = f.config.root.parent().unwrap().to_path_buf();
    let desktop = home.join(".openhuman").join("saas");
    std::fs::create_dir_all(&desktop).unwrap();
    // Through a symlink, and through `..`.
    let link = home.join("alias");
    std::os::unix::fs::symlink(&desktop, &link).unwrap();
    let dotted = home.join("saas").join("..").join(".openhuman").join("saas");
    for root in [link, dotted] {
        let config = SaasConfig::new(root.clone());
        let mut i = inputs(&f, &[]);
        i.config = &config;
        i.home = Some(home.clone());
        let found = violations(&i);
        assert!(
            found
                .iter()
                .any(|v| matches!(v, Violation::Root(why) if why.contains(".openhuman"))),
            "{}: {found:?}",
            root.display()
        );
    }
}

#[test]
fn an_uppercase_host_network_or_an_infinite_cpu_limit_is_refused() {
    let mut f = fixture();
    f.config.tool_allowlist = vec!["host_shell".into()];
    f.config.sandbox.network = "HOST".into();
    f.config.sandbox.cpu_limit = f64::INFINITY;
    let mut i = inputs(&f, &[]);
    i.sandbox_available = true;
    let found = violations(&i);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn a_single_node_needs_no_shared_backend() {
    let mut f = fixture();
    f.config.storage_url = Some("memory:".into());
    assert_eq!(check(&inputs(&f, &[])), Ok(()));
}

#[test]
fn a_clustered_node_needs_a_backend_with_cross_process_cas() {
    let mut f = fixture();
    f.config.advertise_url = Some("http://10.0.0.1:7788".into());
    let found = violations(&inputs(&f, &[]));
    assert!(
        matches!(found.as_slice(), [Violation::Cluster(why)] if why.contains("no storage_url")),
        "{found:?}"
    );

    f.config.storage_url = Some("memory:".into());
    let found = violations(&inputs(&f, &[]));
    assert!(
        matches!(found.as_slice(), [Violation::Cluster(why)] if why.contains("`memory`")),
        "{found:?}"
    );

    // The environment wins over the operator file.
    let env = env(&[("OPENHUMAN_STORAGE_URL", "sqlite:/var/lib/oh/shared.db")]);
    assert_eq!(check(&inputs(&f, &env)), Ok(()));
    f.config.storage_url = Some("mongodb://user:secret@db.internal/oh".into());
    assert_eq!(check(&inputs(&f, &[])), Ok(()));
}

#[test]
fn a_cluster_violation_never_echoes_the_url() {
    let mut f = fixture();
    f.config.advertise_url = Some("http://10.0.0.1:7788".into());
    f.config.storage_url = Some("file:///srv/user:secret@x".into());
    let message = check(&inputs(&f, &[])).unwrap_err().to_string();
    assert!(!message.contains("secret"), "{message}");
}
