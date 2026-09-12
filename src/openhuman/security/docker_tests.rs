use super::*;

#[test]
fn docker_sandbox_name() {
    let sandbox = DockerSandbox::default();
    assert_eq!(sandbox.name(), "docker");
}

#[test]
fn docker_sandbox_default_image() {
    let sandbox = DockerSandbox::default();
    assert_eq!(sandbox.image, "alpine:latest");
}

#[test]
fn docker_with_custom_image() {
    let result = DockerSandbox::with_image("ubuntu:latest".to_string());
    match result {
        Ok(sandbox) => assert_eq!(sandbox.image, "ubuntu:latest"),
        Err(_) => assert!(!DockerSandbox::is_installed()),
    }
}

// ── §1.1 Sandbox isolation flag tests ──────────────────────

#[test]
fn docker_wrap_command_includes_isolation_flags() {
    let sandbox = DockerSandbox::default();
    let mut cmd = Command::new("echo");
    cmd.arg("hello");
    sandbox.wrap_command(&mut cmd).unwrap();

    assert_eq!(
        cmd.get_program().to_string_lossy(),
        "docker",
        "wrapped command should use docker as program"
    );

    let args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    assert!(
        args.contains(&"run".to_string()),
        "must include 'run' subcommand"
    );
    assert!(
        args.contains(&"--rm".to_string()),
        "must include --rm for auto-cleanup"
    );
    assert!(
        args.contains(&"--network".to_string()),
        "must include --network flag"
    );
    assert!(
        args.contains(&"none".to_string()),
        "network must be set to 'none' for isolation"
    );
    assert!(
        args.contains(&"--memory".to_string()),
        "must include --memory limit"
    );
    assert!(
        args.contains(&"512m".to_string()),
        "memory limit must be 512m"
    );
    assert!(
        args.contains(&"--cpus".to_string()),
        "must include --cpus limit"
    );
    assert!(args.contains(&"1.0".to_string()), "CPU limit must be 1.0");
}

#[test]
fn docker_wrap_command_preserves_original_command() {
    let sandbox = DockerSandbox::default();
    let mut cmd = Command::new("ls");
    cmd.arg("-la");
    sandbox.wrap_command(&mut cmd).unwrap();

    let args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    assert!(
        args.contains(&"alpine:latest".to_string()),
        "must include the container image"
    );
    assert!(
        args.contains(&"ls".to_string()),
        "original program must be passed as argument"
    );
    assert!(
        args.contains(&"-la".to_string()),
        "original args must be preserved"
    );
}

#[test]
fn docker_wrap_command_uses_custom_image() {
    let sandbox = DockerSandbox {
        image: "ubuntu:22.04".to_string(),
    };
    let mut cmd = Command::new("echo");
    sandbox.wrap_command(&mut cmd).unwrap();

    let args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    assert!(
        args.contains(&"ubuntu:22.04".to_string()),
        "must use the custom image"
    );
}
