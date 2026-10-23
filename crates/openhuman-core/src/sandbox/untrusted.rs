//! The one gate for shell spawns whose argv the host did not author (issue
//! #7328, phase 5).
//!
//! A scheduled shell job is text a model, a flow or an operator typed into a
//! schedule long before it runs. The security module plans it onto an executor that
//! meets the isolation floor for untrusted code (the TinyBox native jail or a
//! container) and refuses it when none does. The operator's switch that turns the
//! sandbox off never reaches untrusted text: the module plans a run on the host only
//! for the trusted operator, so an untrusted spawn is planned onto a real executor
//! whatever the switch says, and under SaaS it runs in the user's container or not at
//! all.
//!
//! `scripts/ci/spawner-inventory.json` names every caller of this gate.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use super::types::{SandboxExecRequest, SandboxExecResult};
use crate::config::Config;

/// One untrusted shell command, already authorized by the security module.
pub(crate) struct UntrustedShell<'a> {
    /// The dispatch that owns the policy context. `None` uses the ambient
    /// context for legacy scheduler and hook callers.
    pub(crate) dispatch_ctx: Option<&'a crate::core::runtime::DispatchCtx>,
    /// Loaded configuration of the scope the command belongs to.
    pub(crate) config: &'a Config,
    /// Shell text.
    pub(crate) command: &'a str,
    /// Working directory; also the jail's writable workspace.
    pub(crate) cwd: &'a Path,
    /// The core's private state directory (per-call scratch is created below).
    pub(crate) state_dir: &'a Path,
    /// Wall-clock budget.
    pub(crate) timeout: Duration,
    /// Keep the shell's own last-stage pipeline status instead of `pipefail`.
    pub(crate) preserve_pipeline_status: bool,
    /// Extra environment variables for the command (none for a scheduled job).
    pub(crate) env: &'a BTreeMap<String, String>,
    /// Bytes written to the command's stdin (a hook's event JSON), if any.
    pub(crate) stdin: Option<&'a [u8]>,
}

/// No extra environment, for callers that add none.
pub(crate) static NO_ENV: BTreeMap<String, String> = BTreeMap::new();

/// Largest stdin the Docker path can carry. The container runs `sh -c <text>`
/// with no stdin channel, so input rides inside the command as base64 and must
/// fit one argument (Linux caps a single argument at 128 KiB).
const DOCKER_STDIN_MAX: usize = 72 * 1024;

/// Wrap `command` so it reads `stdin` as its standard input.
fn with_inline_stdin(command: &str, stdin: &[u8]) -> anyhow::Result<String> {
    use base64::Engine as _;
    if stdin.len() > DOCKER_STDIN_MAX {
        anyhow::bail!(
            "stdin of {} bytes exceeds the {DOCKER_STDIN_MAX}-byte limit of the Docker sandbox",
            stdin.len()
        );
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(stdin);
    Ok(format!(
        "printf '%s' '{encoded}' | base64 -d | (\n{command}\n)"
    ))
}

/// Run `spawn` under the isolation floor.
///
/// Errors are refusals, never a fallback: a spawn the module will not plan yields
/// [`super::ops::IsolationFloor`], and so does an executor that is unavailable.
pub(crate) async fn run(spawn: UntrustedShell<'_>) -> anyhow::Result<SandboxExecResult> {
    use super::ops::Executor;

    let UntrustedShell {
        dispatch_ctx,
        config,
        command,
        cwd,
        state_dir,
        timeout,
        preserve_pipeline_status,
        env,
        stdin,
    } = spawn;
    // The scheduler has no dispatch context of its own; the task's ambient one
    // answers the component question (#7801 transitional).
    let ambient_ctx;
    let ctx = match dispatch_ctx {
        Some(ctx) => ctx,
        None => {
            ambient_ctx = crate::core::runtime::DispatchCtx::ambient();
            &ambient_ctx
        }
    };
    let operator_off = super::operator::sandbox_off(ctx, config);
    let policy = super::policy::command_policy(config, cwd, state_dir)
        .map_err(|error| anyhow::anyhow!(error))?;
    // The module binds the command text and a digest of its stdin. A container has no
    // stdin channel, so it runs a deterministic wrapper of exactly those two inputs.
    let text = SpawnText {
        command,
        cwd,
        env,
        stdin,
        preserve_pipeline_status,
    };
    let admitted = admit(config, &policy, &text, operator_off).await?;
    let extra_env: HashMap<OsString, OsString> = env
        .iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect();
    match admitted.executor {
        Executor::Host => {
            tracing::debug!("[sandbox:untrusted] running on the host as the module planned");
            crate::config::ensure_usable_cwd(cwd)?;
            std::fs::create_dir_all(state_dir).map_err(|error| {
                anyhow::anyhow!("Cannot prepare sandbox state directory: {error}")
            })?;
            super::shell_exec::run_host_with_stdin(
                config,
                command,
                cwd,
                state_dir,
                env.clone(),
                timeout,
                preserve_pipeline_status,
                None,
                stdin.map(<[u8]>::to_vec),
            )
            .await
        }
        Executor::Jail => {
            crate::config::ensure_usable_cwd(cwd)?;
            tracing::debug!("[sandbox:untrusted] running under the native jail");
            super::native_jail::execute_with_stdin(
                config,
                &admitted.policy,
                command,
                cwd,
                &extra_env,
                timeout,
                admitted.jail,
                preserve_pipeline_status,
                None,
                stdin.map(<[u8]>::to_vec),
            )
            .await
        }
        Executor::Docker => {
            if admitted.policy.mount_workspace {
                crate::config::ensure_usable_cwd(&admitted.policy.workspace_root)?;
            }
            tracing::debug!("[sandbox:untrusted] running in the sandbox container");
            let command = match stdin {
                Some(stdin) => with_inline_stdin(command, stdin)?,
                None => command.to_owned(),
            };
            let request = SandboxExecRequest {
                command,
                working_dir: cwd.to_path_buf(),
                env: extra_env,
                timeout,
            };
            super::docker::docker_exec(config, &admitted.policy, &request).await
        }
    }
}

/// The concrete text of one untrusted spawn, as the module binds it.
struct SpawnText<'a> {
    command: &'a str,
    cwd: &'a Path,
    env: &'a BTreeMap<String, String>,
    stdin: Option<&'a [u8]>,
    preserve_pipeline_status: bool,
}

/// Have the module plan and admit an untrusted spawn. Text the host did not author
/// carries the module's mandatory-isolation floor, whatever the operator's switches say.
#[cfg(feature = "security-module")]
async fn admit(
    config: &Config,
    policy: &super::types::SandboxPolicy,
    spawn: &SpawnText<'_>,
    operator_off: bool,
) -> anyhow::Result<super::ops::Admitted> {
    let operation = super::spawn_plan::SpawnOperation {
        command: spawn.command,
        cwd: spawn.cwd,
        env: spawn
            .env
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        stdin: spawn.stdin,
        preserve_pipeline_status: spawn.preserve_pipeline_status,
    };
    super::ops::admit_planned(
        config,
        policy,
        super::spawn_plan::SpawnSource::Untrusted,
        &operation,
        operator_off,
    )
    .await
}

/// Without the module there is no plan to ask for, and nothing runs unplanned.
#[cfg(not(feature = "security-module"))]
async fn admit(
    _config: &Config,
    _policy: &super::types::SandboxPolicy,
    _spawn: &SpawnText<'_>,
    _operator_off: bool,
) -> anyhow::Result<super::ops::Admitted> {
    anyhow::bail!("the security module is disabled; untrusted spawns are refused")
}

#[cfg(test)]
#[path = "untrusted_tests.rs"]
mod tests;
