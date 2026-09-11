use super::*;

#[test]
fn firejail_sandbox_name() {
    assert_eq!(FirejailSandbox.name(), "firejail");
}

#[test]
fn firejail_description_mentions_dependency() {
    let desc = FirejailSandbox.description();
    assert!(desc.contains("firejail"));
}

#[test]
fn firejail_new_fails_if_not_installed() {
    // This will fail unless firejail is actually installed
    let result = FirejailSandbox::new();
    match result {
        Ok(_) => println!("Firejail is installed"),
        Err(e) => assert!(
            e.kind() == std::io::ErrorKind::NotFound || e.kind() == std::io::ErrorKind::Unsupported
        ),
    }
}

#[test]
fn firejail_wrap_command_prepends_firejail() {
    let sandbox = FirejailSandbox;
    let mut cmd = Command::new("echo");
    cmd.arg("test");

    // Note: wrap_command will fail if firejail isn't installed,
    // but we can still test the logic structure
    let _ = sandbox.wrap_command(&mut cmd);

    // After wrapping, the program should be firejail
    if sandbox.is_available() {
        assert_eq!(cmd.get_program().to_string_lossy(), "firejail");
    }
}

// ── §1.1 Sandbox isolation flag tests ──────────────────────

#[test]
fn firejail_wrap_command_includes_all_security_flags() {
    let sandbox = FirejailSandbox;
    let mut cmd = Command::new("echo");
    cmd.arg("test");
    sandbox.wrap_command(&mut cmd).unwrap();

    assert_eq!(
        cmd.get_program().to_string_lossy(),
        "firejail",
        "wrapped command should use firejail as program"
    );

    let args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    let expected_flags = [
        "--private=home",
        "--private-dev",
        "--nosound",
        "--no3d",
        "--novideo",
        "--nowheel",
        "--notv",
        "--noprofile",
        "--quiet",
    ];

    for flag in &expected_flags {
        assert!(
            args.contains(&flag.to_string()),
            "must include security flag: {flag}"
        );
    }
}

#[test]
fn firejail_wrap_command_preserves_original_command() {
    let sandbox = FirejailSandbox;
    let mut cmd = Command::new("ls");
    cmd.arg("-la");
    cmd.arg("/workspace");
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
        args.contains(&"/workspace".to_string()),
        "original args must be preserved"
    );
}
