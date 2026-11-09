//! Host sandbox routing and environment pass-through characterization.
//! These tests exercise the core adapter without starting a sandbox backend.

use std::path::Path;

use tinyagents_definition::SandboxMode;

use openhuman_core::{
    config::{RuntimeConfig, SandboxConfig},
    sandbox::{policy::route_for_mode, SandboxBackendKind, SandboxPolicy},
};

const ACTION: &str = "/work/action";
const STATE: &str = "/work/state";

fn resolve(runtime: &RuntimeConfig, remote: bool) -> SandboxPolicy {
    route_for_mode(
        SandboxMode::Sandboxed,
        Path::new(ACTION),
        Path::new(STATE),
        &SandboxConfig::default(),
        runtime,
        remote,
    )
}

#[test]
fn resolver_selects_executor_and_network_policy_for_the_turn_mode() {
    let local = RuntimeConfig::default();
    let docker = RuntimeConfig {
        kind: "docker".into(),
        ..RuntimeConfig::default()
    };
    // (runtime, remote) -> (named executor, allow_network)
    let cases = [
        (&local, false, SandboxBackendKind::Local, true),
        (&local, true, SandboxBackendKind::Docker, false),
        (&docker, false, SandboxBackendKind::Docker, true),
        (&docker, true, SandboxBackendKind::Docker, false),
    ];
    for (runtime, remote, backend, network) in cases {
        let policy = resolve(runtime, remote);
        let label = format!("kind={} remote={remote}", runtime.kind);
        assert_eq!(policy.backend, backend, "{label}");
        assert_eq!(policy.allow_network, network, "{label}");
        assert!(policy.docker_overrides.is_some(), "{label}");
        assert_eq!(policy.workspace_root, Path::new(ACTION), "{label}");
        assert_eq!(policy.state_dir, Path::new(STATE), "{label}");
        assert!(
            policy.read_only_mounts.is_empty() && policy.read_write_mounts.is_empty(),
            "{label}"
        );
    }
}

#[test]
fn env_passthrough_is_the_pinned_safe_allowlist() {
    let policy = resolve(&RuntimeConfig::default(), false);
    assert_eq!(
        policy.env_passthrough,
        ["PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR"]
    );
    for secret in [
        "AWS_SECRET_ACCESS_KEY",
        "OPENHUMAN_CORE_TOKEN",
        "GITHUB_TOKEN",
    ] {
        assert!(
            !policy.env_passthrough.iter().any(|v| v == secret),
            "{secret}"
        );
    }
}
