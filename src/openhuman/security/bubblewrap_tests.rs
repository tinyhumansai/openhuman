use super::*;

#[test]
fn bubblewrap_sandbox_name() {
    let sandbox = BubblewrapSandbox;
    assert_eq!(sandbox.name(), "bubblewrap");
}

#[test]
fn bubblewrap_is_available_only_if_installed() {
    // Result depends on whether bwrap is installed
    let sandbox = BubblewrapSandbox;
    let _available = sandbox.is_available();

    // Either way, the name should still work
    assert_eq!(sandbox.name(), "bubblewrap");
}

// ── §1.1 Sandbox isolation flag tests ──────────────────────

#[test]
fn bubblewrap_wrap_command_includes_isolation_flags() {
    let sandbox = BubblewrapSandbox;
    let mut cmd = Command::new("echo");
    cmd.arg("hello");
    sandbox.wrap_command(&mut cmd).unwrap();

    assert_eq!(
        cmd.get_program().to_string_lossy(),
        "bwrap",
        "wrapped command should use bwrap as program"
    );

    let args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    assert!(
        args.contains(&"--unshare-all".to_string()),
        "must include --unshare-all for namespace isolation"
    );
    assert!(
        args.contains(&"--die-with-parent".to_string()),
        "must include --die-with-parent to prevent orphan processes"
    );
    assert!(
        !args.contains(&"--share-net".to_string()),
        "must NOT include --share-net (network should be blocked)"
    );
}

#[test]
fn bubblewrap_wrap_command_preserves_original_command() {
    let sandbox = BubblewrapSandbox;
    let mut cmd = Command::new("ls");
    cmd.arg("-la");
    cmd.arg("/tmp");
    sandbox.wrap_command(&mut cmd).unwrap();

    let args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    assert!(
        args.contains(&"ls".to_string()),
        "original program must be passed as argument"
    );
    assert!(
        args.contains(&"-la".to_string()),
        "original args must be preserved"
    );
    assert!(
        args.contains(&"/tmp".to_string()),
        "original args must be preserved"
    );
}

#[test]
fn bubblewrap_wrap_command_binds_required_paths() {
    let sandbox = BubblewrapSandbox;
    let mut cmd = Command::new("echo");
    sandbox.wrap_command(&mut cmd).unwrap();

    let args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    assert!(
        args.contains(&"--ro-bind".to_string()),
        "must include read-only bind for /usr"
    );
    assert!(
        args.contains(&"--dev".to_string()),
        "must include /dev mount"
    );
    assert!(
        args.contains(&"--proc".to_string()),
        "must include /proc mount"
    );
}

#[test]
fn bubblewrap_wrap_command_uses_private_tmpfs_not_host_tmp() {
    let sandbox = BubblewrapSandbox;
    let mut cmd = Command::new("echo");
    sandbox.wrap_command(&mut cmd).unwrap();

    let args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    // The sandbox must mount a private tmpfs over /tmp ...
    let tmpfs_pos = args.iter().position(|a| a == "--tmpfs");
    assert!(
        tmpfs_pos.is_some(),
        "must include --tmpfs for a private /tmp"
    );
    assert_eq!(
        args.get(tmpfs_pos.unwrap() + 1).map(String::as_str),
        Some("/tmp"),
        "--tmpfs must target /tmp"
    );

    // ... and must NOT read-write bind the host /tmp into the sandbox.
    let has_tmp_rw_bind = args
        .windows(3)
        .any(|w| w[0] == "--bind" && w[1] == "/tmp" && w[2] == "/tmp");
    assert!(
        !has_tmp_rw_bind,
        "must NOT bind host /tmp read-write into the sandbox"
    );
}
