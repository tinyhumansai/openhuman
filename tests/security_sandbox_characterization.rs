//! Phase-0 sandbox characterization (issue #7328, plan Task 1): the precedence
//! `resolve_sandbox_policy` and `command_requires_sandbox` apply today, which the
//! tinybox-backed module path must keep. The SaaS rule (every command sandboxed
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
    config::{Config, RuntimeConfig, SandboxBackend},
    core::runtime::DispatchCtx,
    sandbox::{
        command_requires_sandbox, create_sandbox_backend, resolve_sandbox_policy,
        SandboxBackendKind, SandboxStatus,
    },
};

const ACTION: &str = "/work/action";
const STATE: &str = "/work/state";

fn resolve(
    mode: SandboxMode,
    runtime: &RuntimeConfig,
    remote: bool,
) -> openhuman_core::sandbox::SandboxPolicy {
    resolve_sandbox_policy(mode, Path::new(ACTION), Path::new(STATE), runtime, remote)
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
    config.sandbox.backend = backend;
    config.sandbox.enabled = enabled;
    config.save().await.unwrap();
    (dir, config)
}

#[test]
fn policy_matrix_without_host_override() {
    let _lock = env_guard::env_lock();
    let _off = env_guard::EnvVarGuard::unset("OPENHUMAN_SANDBOX");
    let local = RuntimeConfig::default();
    let docker = RuntimeConfig {
        kind: "docker".into(),
        ..RuntimeConfig::default()
    };
    // (mode, runtime, remote) -> (backend, allow_network, docker overrides present)
    let cases = [
        (
            SandboxMode::None,
            &local,
            false,
            SandboxBackendKind::None,
            true,
            false,
        ),
        (
            SandboxMode::None,
            &local,
            true,
            SandboxBackendKind::None,
            true,
            false,
        ),
        (
            SandboxMode::ReadOnly,
            &local,
            false,
            SandboxBackendKind::None,
            true,
            false,
        ),
        (
            SandboxMode::Sandboxed,
            &local,
            false,
            SandboxBackendKind::Local,
            true,
            false,
        ),
        (
            SandboxMode::Sandboxed,
            &local,
            true,
            SandboxBackendKind::Docker,
            false,
            true,
        ),
        (
            SandboxMode::Sandboxed,
            &docker,
            false,
            SandboxBackendKind::Docker,
            true,
            true,
        ),
        (
            SandboxMode::Sandboxed,
            &docker,
            true,
            SandboxBackendKind::Docker,
            false,
            true,
        ),
    ];
    for (mode, runtime, remote, backend, network, overrides) in cases {
        let policy = resolve(mode, runtime, remote);
        let label = format!("{mode:?} kind={} remote={remote}", runtime.kind);
        assert_eq!(policy.backend, backend, "{label}");
        assert_eq!(policy.allow_network, network, "{label}");
        assert_eq!(policy.docker_overrides.is_some(), overrides, "{label}");
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
    let policy = resolve(SandboxMode::Sandboxed, &RuntimeConfig::default(), false);
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

#[test]
fn host_off_switch_unconfines_only_sandboxed_mode() {
    let _lock = env_guard::env_lock();
    for spelling in ["off", "OFF", " none ", "0", "false", "disabled"] {
        let _env = env_guard::EnvVarGuard::set("OPENHUMAN_SANDBOX", spelling);
        let policy = resolve(SandboxMode::Sandboxed, &RuntimeConfig::default(), true);
        assert_eq!(policy.backend, SandboxBackendKind::None, "{spelling:?}");
    }
    for spelling in ["on", "", "1", "local", "yes"] {
        let _env = env_guard::EnvVarGuard::set("OPENHUMAN_SANDBOX", spelling);
        let policy = resolve(SandboxMode::Sandboxed, &RuntimeConfig::default(), false);
        assert_eq!(policy.backend, SandboxBackendKind::Local, "{spelling:?}");
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
    // The host off switch beats the agent's own request.
    {
        let _off = env_guard::EnvVarGuard::set("OPENHUMAN_SANDBOX", "off");
        assert!(!command_requires_sandbox(
            &DispatchCtx::process_default(),
            Some(SandboxMode::Sandboxed)
        )
        .await
        .unwrap());
    }
    drop(_ws);
    drop(dir);

    // An explicit operator backend requires a sandbox even for a default agent...
    for backend in [
        SandboxBackend::Landlock,
        SandboxBackend::Firejail,
        SandboxBackend::Bubblewrap,
        SandboxBackend::Docker,
    ] {
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

    // ...unless the legacy `enabled = false` flag turns isolation off.
    let (dir, _config) = scratch_config(SandboxBackend::Docker, Some(false)).await;
    let _ws = env_guard::EnvVarGuard::set_path("OPENHUMAN_WORKSPACE", dir.path());
    let _off = env_guard::EnvVarGuard::unset("OPENHUMAN_SANDBOX");
    assert!(
        !command_requires_sandbox(&DispatchCtx::process_default(), None)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn backend_handles_report_a_defined_status() {
    let _lock = env_guard::env_lock_async().await;
    let none = create_sandbox_backend(
        &DispatchCtx::process_default(),
        &resolve(SandboxMode::None, &RuntimeConfig::default(), false),
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
        &resolve(SandboxMode::Sandboxed, &RuntimeConfig::default(), false),
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
