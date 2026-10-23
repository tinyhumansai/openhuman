//! The stdio jail is host policy: where it lives, what it grants, what it denies.
use super::*;

fn config_in(dir: &Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = dir.to_path_buf();
    config
}

#[test]
fn the_jail_is_rooted_in_a_directory_of_its_own_under_the_workspace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = config_in(temp.path());
    let sandbox = for_config(&config).expect("a sandbox");
    assert_eq!(
        Path::new(&sandbox.workspace),
        temp.path().join("mcp").join("stdio")
    );
    assert!(Path::new(&sandbox.workspace).is_dir());
    assert_ne!(Path::new(&sandbox.workspace), temp.path());
}

#[test]
fn the_jail_requires_the_untrusted_code_isolation_floor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sandbox = for_config(&config_in(temp.path())).expect("a sandbox");
    assert!(sandbox.require_process_isolation);
    assert!(sandbox.allow_net && sandbox.allow_subprocess);
}

#[test]
fn only_existing_paths_are_granted_and_operator_extras_are_included() {
    let temp = tempfile::tempdir().expect("tempdir");
    let extra = tempfile::tempdir().expect("extra");
    let mut config = config_in(temp.path());
    config.runtime.local_jail.extra_read_only = vec![
        extra.path().display().to_string(),
        "/openhuman/definitely/missing".into(),
    ];
    config.runtime.local_jail.extra_read_write = vec![extra.path().display().to_string()];
    let sandbox = for_config(&config).expect("a sandbox");
    assert!(sandbox
        .read_only
        .contains(&extra.path().display().to_string()));
    assert!(sandbox
        .read_write
        .contains(&extra.path().display().to_string()));
    assert!(
        sandbox
            .read_only
            .iter()
            .chain(&sandbox.read_write)
            .all(|path| Path::new(path).exists()),
        "TinyBox rejects a grant it cannot canonicalize"
    );
    let unique: std::collections::HashSet<_> = sandbox.read_only.iter().collect();
    assert_eq!(unique.len(), sandbox.read_only.len());
}

#[test]
fn credential_stores_are_denied_when_a_home_exists() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sandbox = for_config(&config_in(temp.path())).expect("a sandbox");
    if let Some(home) = dirs::home_dir() {
        assert!(
            sandbox
                .deny_read
                .contains(&home.join(".ssh").display().to_string()),
            "{:?}",
            sandbox.deny_read
        );
    }
}

#[test]
fn an_uncreatable_workspace_yields_no_sandbox_so_servers_are_refused() {
    let temp = tempfile::tempdir().expect("tempdir");
    // A file where the `mcp` directory must go makes creation fail.
    std::fs::write(temp.path().join("mcp"), b"not a directory").expect("blocker");
    assert!(for_config(&config_in(temp.path())).is_none());
}

#[test]
fn prepared_toolchain_grants_widen_only_that_workspaces_jail() {
    let temp = tempfile::tempdir().expect("tempdir");
    let other = tempfile::tempdir().expect("other");
    let toolchain = tempfile::tempdir().expect("toolchain");
    let config = config_in(temp.path());
    prepared().lock().expect("lock").insert(
        config.workspace_dir.clone(),
        JailGrants {
            read_only: vec![toolchain.path().to_path_buf()],
            read_write: Vec::new(),
        },
    );
    let widened = for_config(&config).expect("a sandbox");
    assert!(widened
        .read_only
        .contains(&toolchain.path().display().to_string()));
    let unrelated = for_config(&config_in(other.path())).expect("a sandbox");
    assert!(!unrelated
        .read_only
        .contains(&toolchain.path().display().to_string()));
}
