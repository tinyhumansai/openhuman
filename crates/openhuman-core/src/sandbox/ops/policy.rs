use crate::core::runtime::DispatchCtx;

use crate::sandbox::{
    docker,
    types::{SandboxBackendHandle, SandboxBackendKind, SandboxPolicy, SandboxStatus},
};

/// Safe environment variables forwarded into sandboxed execution.
///
/// This is the Docker container's host-environment allowlist and the set the
/// TinyBox module's own safe snapshot starts from for every native shell child
/// ([`execute_unsandboxed`] and the native jail adapter): the module reads these
/// names from the parent and adds the Windows process-bootstrap variables
/// itself. Keeping the bootstrap names out of this policy prevents Windows paths
/// from being passed into Linux Docker containers.
pub const SANDBOX_ENV_PASSTHROUGH: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];

/// Host switch that turns the agent sandbox off for the whole process.
///
/// For hosts that already isolate the core (a container, a CI or benchmark
/// task image, a VM): the OS jail confines commands to the action dir, which
/// breaks work such as installing packages or editing `/etc` that the outer
/// isolation already permits. `off`, `none`, `0`, `false` or `disabled`
/// (case-insensitive) disable it; anything else, or unset, leaves it on.
pub const SANDBOX_OFF_ENV: &str = "OPENHUMAN_SANDBOX";

/// Whether `value` (the `OPENHUMAN_SANDBOX` setting) disables the sandbox.
pub fn sandbox_off_value(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("off" | "none" | "0" | "false" | "disabled")
    )
}
pub(crate) fn sandbox_disabled_by_host(ctx: &DispatchCtx) -> bool {
    sandbox_off_value(std::env::var(SANDBOX_OFF_ENV).ok().as_deref()) || sandbox_component_off(ctx)
}

/// `[components] sandbox = false` in the context `ctx` runs under. With no
/// context the component is on, since the set only narrows.
pub(crate) fn sandbox_component_off(ctx: &DispatchCtx) -> bool {
    let off = !ctx.components().sandbox;
    if off {
        tracing::debug!("[sandbox] component 'sandbox' disabled, using the unsandboxed fallback");
    }
    off
}

/// The saved config says no isolation: the legacy `enabled = false` flag or
/// `[components] sandbox = false`.
pub(crate) fn config_disables_sandbox(config: &crate::config::Config) -> bool {
    config.sandbox.enabled == Some(false)
        || !crate::core::runtime::ComponentSet::from_config(&config.components).sandbox
}

/// Create a backend handle for the resolved policy. For Docker this
/// checks availability; for Local it checks the OS backend.
/// The status a `Local` sandbox handle reports, given the backend `pick_backend`
/// actually chose.
///
/// Extracted as a free function so the decision is testable on **any** host.
/// Asserting it through `create_sandbox_backend` cannot work: which branch runs
/// depends on whether the machine has an OS jail, so on a host with Seatbelt or
/// Landlock the noop path is never reached and a regression to "always Ready"
/// passes unnoticed. That is exactly how the original defect survived.
#[cfg(any(feature = "modules", test))]
pub(crate) fn local_status_for_jail(
    isolation: Option<crate::sandbox::types::SandboxIsolation>,
) -> SandboxStatus {
    if isolation.is_some() {
        SandboxStatus::Ready
    } else {
        SandboxStatus::Inactive
    }
}

pub async fn create_sandbox_backend(
    ctx: &DispatchCtx,
    policy: &SandboxPolicy,
) -> SandboxBackendHandle {
    if policy.backend != SandboxBackendKind::Docker {
        return create_sandbox_backend_with_config(policy, &crate::config::Config::default())
            .await
            .unwrap_or(SandboxBackendHandle {
                kind: policy.backend,
                status: SandboxStatus::Error,
                backend_id: None,
                isolation: None,
            });
    }
    let config = match ctx.load_config().await {
        Ok(config) => config,
        Err(_) if policy.backend == SandboxBackendKind::Docker => {
            return docker_error_handle();
        }
        Err(_) => crate::config::Config::default(),
    };
    create_sandbox_backend_with_config(policy, &config)
        .await
        .unwrap_or_else(|_| docker_error_handle())
}

/// Resolve backend status with the caller's loaded configuration.
pub async fn create_sandbox_backend_with_config(
    policy: &SandboxPolicy,
    config: &crate::config::Config,
) -> Result<SandboxBackendHandle, String> {
    match policy.backend {
        SandboxBackendKind::None => Ok(SandboxBackendHandle {
            kind: SandboxBackendKind::None,
            status: SandboxStatus::Ready,
            backend_id: None,
            isolation: None,
        }),
        SandboxBackendKind::Local => {
            let (backend_name, status, isolation) =
                crate::sandbox::native_jail::status(config).await?;
            if status == SandboxStatus::Inactive {
                tracing::warn!(
                    backend = backend_name,
                    "[sandbox:local] TinyBox native jail is unavailable or below the required isolation floor"
                );
            }
            Ok(SandboxBackendHandle {
                kind: SandboxBackendKind::Local,
                status,
                backend_id: Some(backend_name),
                isolation,
            })
        }
        SandboxBackendKind::Docker => docker::docker_backend_handle(config).await,
        // A box is created on a tenant's first command, so there is nothing
        // to probe or name yet; the gateway's availability shows on that call.
        SandboxBackendKind::OpenShell => Ok(SandboxBackendHandle {
            kind: SandboxBackendKind::OpenShell,
            status: SandboxStatus::Ready,
            backend_id: None,
            isolation: None,
        }),
    }
}

fn docker_error_handle() -> SandboxBackendHandle {
    SandboxBackendHandle {
        kind: SandboxBackendKind::Docker,
        status: SandboxStatus::Error,
        backend_id: None,
        isolation: None,
    }
}
