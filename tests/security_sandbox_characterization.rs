//! Phase-0 sandbox characterization (issue #7328, plan Task 1): the request side
//! (`spawn_policy` and `command_requires_sandbox`), which the tinybox-backed module path
//! keeps. Which executor runs a spawn, and whether the operator's off switch skips the
//! jail, is the security module's decision and is pinned in
//! `sandbox/spawn_plan/integration_tests.rs`. The SaaS rule (every command sandboxed
//! even with `OPENHUMAN_SANDBOX=off`) needs a process locked to SaaS mode, which
//! only a SaaS boot can do, so it is pinned beside the code in
//! `sandbox/ops_tests.rs` (`saas_requires_the_sandbox_even_when_the_host_turns_it_off`).
//! The `Noop => Inactive` status mapping is pinned in `sandbox/native_jail_tests.rs`.

#[path = "support/env_guard.rs"]
mod env_guard;
#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

use std::path::Path;

use tinyagents_definition::SandboxMode;

use openhuman_core::{
    config::{Config, RuntimeConfig, SandboxBackend, SandboxConfig},
    core::runtime::DispatchCtx,
    sandbox::{
        command_requires_sandbox, create_sandbox_backend, spawn_policy, SandboxBackendKind,
        SandboxStatus,
    },
};

const ACTION: &str = "/work/action";
const STATE: &str = "/work/state";

fn resolve(runtime: &RuntimeConfig, remote: bool) -> openhuman_core::sandbox::SandboxPolicy {
    spawn_policy(
        Path::new(ACTION),
        Path::new(STATE),
        &SandboxConfig::default(),
        runtime,
        remote,
    )
}

async fn scratch_config(
    backend: SandboxBackend,
    enabled: Option<bool>,
) -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.config_path = dir.path().join("config.toml");
    config.action_dir = dir.path().join("action");
    config.security.sandbox.backend = backend;
    config.security.sandbox.enabled = enabled;
    config.save().await.unwrap();
    (dir, config)
}

#[test]
fn policy_matrix_names_an_executor_only_when_the_configuration_does() {
    let local = RuntimeConfig::default();
    let docker = RuntimeConfig {
        kind: "docker".into(),
        ..RuntimeConfig::default()
    };
    // (runtime, remote) -> (named executor, allow_network)
    let cases = [
        (&local, false, SandboxBackendKind::None, true),
        (&local, true, SandboxBackendKind::Docker, false),
        (&docker, false, SandboxBackendKind::Docker, true),
        (&docker, true, SandboxBackendKind::Docker, false),
    ];
    for (runtime, remote, backend, network) in cases {
        let policy = resolve(runtime, remote);
        let label = format!("kind={} remote={remote}", runtime.kind);
        assert_eq!(policy.backend, backend, "{label}");
        assert_eq!(policy.allow_network, network, "{label}");
        assert!(
            policy.docker_overrides.is_some(),
            "{label}: the container settings travel whichever executor is chosen"
        );
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

#[tokio::test]
async fn command_requires_sandbox_precedence_for_single_user_hosts() {
    let _lock = env_guard::env_lock_async().await;
    tinyhumans_boot::boot();
    let _keyring = env_guard::EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let _storage = env_guard::EnvVarGuard::unset("OPENHUMAN_STORAGE_URL");

    // Default config: only an agent that asks for a sandbox gets one.
    let (dir, _config) = scratch_config(SandboxBackend::Auto, None).await;
    let _ws = env_guard::EnvVarGuard::set_path("OPENHUMAN_WORKSPACE", dir.path());
    {
        let _off = env_guard::EnvVarGuard::unset("OPENHUMAN_SANDBOX");
        assert!(
            !command_requires_sandbox(&DispatchCtx::process_default(), None)
                .await
                .unwrap()
        );
        assert!(!command_requires_sandbox(
            &DispatchCtx::process_default(),
            Some(SandboxMode::None)
        )
        .await
        .unwrap());
        assert!(!command_requires_sandbox(
            &DispatchCtx::process_default(),
            Some(SandboxMode::ReadOnly)
        )
        .await
        .unwrap());
        assert!(command_requires_sandbox(
            &DispatchCtx::process_default(),
            Some(SandboxMode::Sandboxed)
        )
        .await
        .unwrap());
    }
    // The off switch no longer routes around the module: a sandboxed request is still
    // planned there, and the module decides whether the jail may be skipped.
    {
        let _off = env_guard::EnvVarGuard::set("OPENHUMAN_SANDBOX", "off");
        assert!(command_requires_sandbox(
            &DispatchCtx::process_default(),
            Some(SandboxMode::Sandboxed)
        )
        .await
        .unwrap());
    }
    drop(_ws);
    drop(dir);

    // An explicit operator backend requires a sandbox even for a default agent...
    for backend in [SandboxBackend::Landlock, SandboxBackend::Docker] {
        let label = format!("{backend:?}");
        let (dir, _config) = scratch_config(backend, None).await;
        let _ws = env_guard::EnvVarGuard::set_path("OPENHUMAN_WORKSPACE", dir.path());
        let _off = env_guard::EnvVarGuard::unset("OPENHUMAN_SANDBOX");
        assert!(
            command_requires_sandbox(&DispatchCtx::process_default(), None)
                .await
                .unwrap(),
            "{label}"
        );
        assert!(
            command_requires_sandbox(&DispatchCtx::process_default(), Some(SandboxMode::None))
                .await
                .unwrap(),
            "{label}"
        );
    }

    // ...also with the legacy `enabled = false` flag: the flag is declared to the module,
    // which plans the run on the host for the trusted operator, rather than the host
    // skipping the plan.
    let (dir, _config) = scratch_config(SandboxBackend::Docker, Some(false)).await;
    let _ws = env_guard::EnvVarGuard::set_path("OPENHUMAN_WORKSPACE", dir.path());
    let _off = env_guard::EnvVarGuard::unset("OPENHUMAN_SANDBOX");
    assert!(
        command_requires_sandbox(&DispatchCtx::process_default(), None)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn backend_handles_report_a_defined_status() {
    let _lock = env_guard::env_lock_async().await;
    let none = create_sandbox_backend(
        &DispatchCtx::process_default(),
        &resolve(&RuntimeConfig::default(), false),
    )
    .await;
    assert_eq!(none.kind, SandboxBackendKind::None);
    assert_eq!(none.status, SandboxStatus::Ready);
    assert!(none.backend_id.is_none());

    // The local jail is Ready where the OS provides one that is suitable for
    // untrusted code and Inactive otherwise (the Noop floor); never Error and
    // never silently Ready without a jail.
    let local = create_sandbox_backend(
        &DispatchCtx::process_default(),
        &resolve(&RuntimeConfig::default(), false),
    )
    .await;
    assert_eq!(local.kind, SandboxBackendKind::Local);
    assert!(
        matches!(local.status, SandboxStatus::Ready | SandboxStatus::Inactive),
        "{:?}",
        local.status
    );
    assert!(local.backend_id.is_some());
}
