use super::*;

#[test]
fn ephemeral_creates_both_dirs_and_removes_them_on_drop() {
    let root;
    {
        let resolved = ResolvedWorkspace::resolve(&Workspace::Ephemeral, None).expect("resolve");
        assert!(resolved.workspace_dir.is_dir());
        assert!(resolved.action_dir.is_dir());
        root = resolved
            .workspace_dir
            .parent()
            .expect("workspace has a parent")
            .to_path_buf();
        assert!(root.is_dir());
    }
    // The TempDir went with the ResolvedWorkspace. If this ever fails, every
    // ephemeral harness is leaking a directory — and on a tmpfs /tmp, RAM.
    assert!(!root.exists(), "ephemeral workspace outlived its owner");
}

#[test]
fn the_action_dir_is_never_inside_the_workspace() {
    // `is_workspace_internal_path` blocks agent writes beneath the workspace
    // fail-closed, so an action_dir nested in it would be an agent that cannot
    // write anywhere — with no error, just refusals.
    let resolved = ResolvedWorkspace::resolve(&Workspace::Ephemeral, None).expect("resolve");
    assert!(
        !resolved.action_dir.starts_with(&resolved.workspace_dir),
        "action_dir {} is inside workspace_dir {}",
        resolved.action_dir.display(),
        resolved.workspace_dir.display()
    );
}

#[test]
fn a_dir_workspace_uses_the_given_path_and_a_sibling_action_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("ws");
    let resolved =
        ResolvedWorkspace::resolve(&Workspace::dir(workspace.clone()), None).expect("resolve");

    assert_eq!(resolved.workspace_dir, workspace);
    assert_eq!(resolved.action_dir, temp.path().join("action"));
    assert!(resolved.workspace_dir.is_dir());
    assert!(resolved.action_dir.is_dir());
    assert!(!resolved.action_dir.starts_with(&resolved.workspace_dir));
}

#[test]
fn a_dir_workspace_persists_after_drop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("ws");
    {
        ResolvedWorkspace::resolve(&Workspace::dir(workspace.clone()), None).expect("resolve");
    }
    assert!(
        workspace.is_dir(),
        "a caller-owned workspace must survive the harness"
    );
}

#[test]
fn an_explicit_action_dir_overrides_the_default() {
    let temp = tempfile::tempdir().expect("tempdir");
    let action = temp.path().join("my-project");
    std::fs::create_dir_all(&action).expect("create");

    let resolved =
        ResolvedWorkspace::resolve(&Workspace::Ephemeral, Some(&action)).expect("resolve");
    assert_eq!(resolved.action_dir, action);
}

#[test]
fn inherit_computes_no_paths_of_its_own() {
    // The operator's resolution chain (OPENHUMAN_WORKSPACE, active_user.toml,
    // per-user scoping) belongs to `Config::load_or_init`. Re-deriving it here
    // would be a second implementation free to drift from it.
    let resolved = ResolvedWorkspace::resolve(&Workspace::Inherit, None).expect("resolve");
    assert_eq!(resolved.workspace_dir, std::path::PathBuf::new());
}

#[test]
fn only_inherit_is_operator_owned() {
    assert!(Workspace::Inherit.is_operator_owned());
    assert!(!Workspace::Ephemeral.is_operator_owned());
    assert!(!Workspace::dir("/tmp/x").is_operator_owned());
}

#[test]
fn the_default_workspace_is_ephemeral() {
    // A library call that wrote into the operator's real install by default
    // would make "try this out" destructive.
    assert!(matches!(Workspace::default(), Workspace::Ephemeral));
}

#[test]
fn the_config_path_sits_beside_the_workspace_not_at_the_default() {
    // Credential state, auth profiles and the keyring file backend resolve
    // against `config_path`'s parent. If it stayed at `~/.openhuman/config.toml`
    // while `workspace_dir` pointed at a temp dir, an "ephemeral" harness would
    // quietly read and write the operator's real credentials.
    let resolved = ResolvedWorkspace::resolve(&Workspace::Ephemeral, None).expect("resolve");
    let state_dir = resolved
        .config_path
        .parent()
        .expect("config_path has a parent");

    assert_eq!(state_dir, resolved.workspace_dir.parent().expect("parent"));
    assert!(
        !state_dir.starts_with(dirs::home_dir().unwrap_or_default().join(".openhuman")),
        "ephemeral credential state leaked into the operator's install: {}",
        state_dir.display()
    );
}

#[test]
fn a_dir_workspace_puts_config_beside_it_too() {
    let temp = tempfile::tempdir().expect("tempdir");
    let resolved =
        ResolvedWorkspace::resolve(&Workspace::dir(temp.path().join("ws")), None).expect("resolve");
    assert_eq!(resolved.config_path, temp.path().join("config.toml"));
}

#[test]
fn a_relative_dir_workspace_does_not_resolve_siblings_against_cwd() {
    // `Path::parent()` on the single-component relative path "ws" is
    // `Some("")`, not `None`. Resolving the sibling action_dir / config_path
    // against that empty parent would put them in the process working
    // directory rather than beside the workspace — breaking the
    // credential-isolation invariant. Both must fall back to the workspace's
    // own directory.
    // Run from a contained cwd so the bare relative path and its sibling
    // directories land in a temp dir rather than the test's working directory.
    let cwd_guard = std::env::current_dir().expect("current dir");
    let temp = tempfile::tempdir().expect("tempdir");
    std::env::set_current_dir(temp.path()).expect("chdir to temp");
    let result = std::panic::catch_unwind(|| {
        let resolved = ResolvedWorkspace::resolve(&Workspace::dir("ws"), None).expect("resolve");
        assert_eq!(resolved.workspace_dir, std::path::PathBuf::from("ws"));
        assert_eq!(
            resolved.action_dir,
            std::path::PathBuf::from("ws").join("action")
        );
        assert_eq!(
            resolved.config_path,
            std::path::PathBuf::from("ws").join("config.toml")
        );
    });
    std::env::set_current_dir(&cwd_guard).expect("restore cwd");
    result.expect("assertions held");
}
