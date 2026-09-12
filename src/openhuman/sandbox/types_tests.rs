use super::*;

#[test]
fn sandbox_exec_result_success_checks() {
    let ok = SandboxExecResult {
        exit_code: 0,
        stdout: "hello".into(),
        stderr: String::new(),
        timed_out: false,
    };
    assert!(ok.success());

    let failed = SandboxExecResult {
        exit_code: 1,
        stdout: String::new(),
        stderr: "error".into(),
        timed_out: false,
    };
    assert!(!failed.success());

    let timeout = SandboxExecResult {
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
        timed_out: true,
    };
    assert!(!timeout.success());
}

#[test]
fn sandbox_backend_kind_default_is_none() {
    assert_eq!(SandboxBackendKind::default(), SandboxBackendKind::None);
}

#[test]
fn sandbox_policy_serializes_roundtrip() {
    let policy = SandboxPolicy {
        backend: SandboxBackendKind::Docker,
        workspace_root: PathBuf::from("/workspace"),
        read_only_mounts: vec![PathBuf::from("/usr/lib")],
        allow_network: false,
        env_passthrough: vec!["PATH".into()],
        docker_overrides: Some(DockerOverrides {
            image: Some("node:20-slim".into()),
            memory_limit_mb: Some(256),
            ..DockerOverrides::default()
        }),
    };
    let json = serde_json::to_string(&policy).unwrap();
    let back: SandboxPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(back.backend, SandboxBackendKind::Docker);
    assert!(!back.allow_network);
}

#[test]
fn elevated_tools_contains_expected_entries() {
    assert!(ELEVATED_TOOLS.contains(&"git_operations"));
    assert!(ELEVATED_TOOLS.contains(&"install_tool"));
    assert!(!ELEVATED_TOOLS.contains(&"shell"));
}

#[test]
fn sandbox_status_variants() {
    assert_ne!(SandboxStatus::Inactive, SandboxStatus::Ready);
    assert_ne!(SandboxStatus::Ready, SandboxStatus::Busy);
    assert_ne!(SandboxStatus::Busy, SandboxStatus::Error);
}
