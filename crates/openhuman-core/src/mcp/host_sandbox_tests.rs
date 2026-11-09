use super::*;

fn config_in(dir: &Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = dir.to_path_buf();
    config
}

#[cfg(feature = "modules")]
fn path_floor_port(
    protected: impl IntoIterator<Item = PathBuf>,
    credentials: Vec<PathBuf>,
) -> crate::modules::security::gap::PathFloorPortGuard {
    crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new(protected, credentials),
    )
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn sandbox_workspace_is_a_dedicated_directory_inside_the_core_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let _port = path_floor_port([], vec![]);
    let sandbox = stdio_sandbox(&config_in(tmp.path()))
        .await
        .expect("a sandbox");
    let expected = tmp.path().join(WORKSPACE_DIR).canonicalize().unwrap();
    assert_eq!(sandbox.workspace, expected.to_string_lossy());
    assert!(expected.is_dir());
    assert!(sandbox.allow_subprocess);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn configured_grants_pass_the_host_floor() {
    let tmp = tempfile::tempdir().unwrap();
    let _port = path_floor_port([PathBuf::from("/etc")], vec![]);
    let granted = tmp.path().join("tools");
    std::fs::create_dir_all(&granted).unwrap();
    let mut config = config_in(tmp.path());
    config.runtime.local_jail.extra_read_only = vec![
        granted.to_string_lossy().into_owned(),
        "/etc".into(),
        "relative/path".into(),
        "/no/such/dir/anywhere".into(),
    ];
    let sandbox = stdio_sandbox(&config).await.expect("a sandbox");
    let granted = granted
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(sandbox.read_only, vec![granted]);
}

#[test]
fn isolation_floor_is_required_except_for_the_macos_desktop() {
    assert!(require_process_isolation(true));
    assert_eq!(require_process_isolation(false), !cfg!(target_os = "macos"));
}

#[test]
fn enforcement_is_declared_only_where_the_host_accepts_a_best_effort_backend() {
    assert_eq!(enforcement(true), StdioSandboxEnforcement::Enforced);
    assert_eq!(enforcement(false), StdioSandboxEnforcement::Declared);
    assert_eq!(
        enforcement(require_process_isolation(true)),
        StdioSandboxEnforcement::Enforced,
        "a SaaS tenant is always enforced"
    );
    let desktop = enforcement(require_process_isolation(false));
    if cfg!(target_os = "macos") {
        assert_eq!(desktop, StdioSandboxEnforcement::Declared);
    } else {
        assert_eq!(desktop, StdioSandboxEnforcement::Enforced);
    }
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn built_sandbox_enforcement_follows_the_isolation_requirement() {
    let tmp = tempfile::tempdir().unwrap();
    let _port = path_floor_port([], vec![]);
    let sandbox = stdio_sandbox(&config_in(tmp.path()))
        .await
        .expect("a sandbox");
    assert_eq!(
        sandbox.enforcement,
        enforcement(sandbox.require_process_isolation)
    );
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn an_unusable_workspace_refuses_stdio_servers_instead_of_running_them_unconfined() {
    let tmp = tempfile::tempdir().unwrap();
    let _port = path_floor_port([], vec![]);
    let file = tmp.path().join("not-a-dir");
    std::fs::write(&file, b"").unwrap();
    assert!(stdio_sandbox(&config_in(&file)).await.is_none());
}

#[tokio::test]
async fn home_relative_grants_expand_and_forbidden_ones_are_refused() {
    let home = tempfile::tempdir().unwrap();
    let _port = path_floor_port(
        [PathBuf::from("/etc"), home.path().to_path_buf()],
        vec![home.path().join(".ssh")],
    );
    std::fs::create_dir_all(home.path().join("tools")).unwrap();
    let raw = vec![
        "~/tools".to_string(),
        "~/tools".to_string(),
        "/etc".to_string(),
        home.path().to_string_lossy().into_owned(),
    ];
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    let admitted = admit_all(&raw, Some(home.path())).await;
    let tools = home.path().join("tools").canonicalize().unwrap();
    assert_eq!(admitted, vec![tools.to_string_lossy().into_owned()]);
}

#[test]
fn merged_grants_are_deduplicated() {
    let mut into = vec!["/a".to_string()];
    merge(
        &mut into,
        &[
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/b"),
        ],
    );
    assert_eq!(into, vec!["/a".to_string(), "/b".to_string()]);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn existing_credential_stores_are_denied_by_their_real_path() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let ssh = home.join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let _port = path_floor_port([], vec![ssh.clone()]);
    let config = config_in(&workspace);
    let denied = credential_denials(&config, Some(&home), &workspace)
        .await
        .unwrap();
    let ssh = ssh.canonicalize().unwrap().to_string_lossy().into_owned();
    assert!(denied.contains(&ssh), "{denied:?}");
    assert_eq!(denied.iter().filter(|d| **d == ssh).count(), 1);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn a_workspace_inside_a_credential_store_is_not_denied_to_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let workspace = home.join(".ssh").join("mcp_stdio");
    std::fs::create_dir_all(&workspace).unwrap();
    let _port = path_floor_port([], vec![home.join(".ssh")]);
    let workspace = workspace.canonicalize().unwrap();
    let config = config_in(&workspace);
    let denied = credential_denials(&config, Some(&home), &workspace)
        .await
        .unwrap();
    let ssh = home.join(".ssh").canonicalize().unwrap();
    assert!(
        !denied.contains(&ssh.to_string_lossy().into_owned()),
        "{denied:?}"
    );
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn toolchain_grants_only_ever_add_to_the_configured_grants() {
    let tmp = tempfile::tempdir().unwrap();
    let _port = path_floor_port([], vec![]);
    let config = config_in(tmp.path());
    let mut sandbox = stdio_sandbox(&config).await.expect("a sandbox");
    let before = sandbox.read_only.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer()
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    with_toolchain_grants(&config, &mut sandbox).await;
    assert!(before.iter().all(|grant| sandbox.read_only.contains(grant)));
}
