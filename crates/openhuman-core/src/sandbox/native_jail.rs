//! Host policy and lifecycle adapters for TinyBox-owned native and passthrough execution.

use std::{collections::HashMap, ffi::OsString, path::Path, time::Duration};

#[cfg(feature = "modules")]
use std::{collections::BTreeMap, ffi::OsStr};

use super::types::{SandboxExecResult, SandboxPolicy, SandboxStatus};
use crate::{agent::platform_shell, config::Config};

#[cfg(feature = "modules")]
use crate::modules::tinybox::{self, contract};

/// Report whether the selected module-owned jail can meet the host's security floor.
#[cfg(feature = "modules")]
pub(crate) async fn status(config: &Config) -> Result<(String, SandboxStatus), String> {
    let status = tinybox::jail_status(config).await?;
    Ok((
        status.backend,
        super::ops::local_status_for_jail(status.available, status.suitable_for_untrusted_code),
    ))
}

#[cfg(not(feature = "modules"))]
pub(crate) async fn status(_config: &Config) -> Result<(String, SandboxStatus), String> {
    Err("TinyBox module support is disabled; native jail status is unavailable".to_owned())
}

/// Verify that TinyBox reports a usable provider before an explicitly selected jail is accepted.
#[cfg(feature = "modules")]
pub(crate) async fn require_backend(config: &Config, expected: &str) -> anyhow::Result<()> {
    let status = tinybox::jail_status(config)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail status is unavailable"))?;
    if !status.available || !status.suitable_for_untrusted_code || status.backend != expected {
        anyhow::bail!("requested native jail backend is unavailable");
    }
    Ok(())
}

#[cfg(not(feature = "modules"))]
pub(crate) async fn require_backend(_config: &Config, _expected: &str) -> anyhow::Result<()> {
    anyhow::bail!("TinyBox module support is disabled; native jail execution is unavailable")
}

/// Run a command inside TinyBox's native jail and keep its resource lifecycle module-owned.
pub(crate) async fn execute(
    config: &Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    explicitly_requested: bool,
) -> anyhow::Result<SandboxExecResult> {
    #[cfg(feature = "modules")]
    {
        let config = config.clone();
        let policy = policy.clone();
        let command = command.to_owned();
        let working_dir = working_dir.to_path_buf();
        let extra_env = extra_env.clone();
        // The worker owns cancellation and resource cleanup if the caller drops its wait.
        tokio::spawn(async move {
            execute_owned(
                &config,
                &policy,
                &command,
                &working_dir,
                &extra_env,
                timeout,
                explicitly_requested,
            )
            .await
        })
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail worker did not finish"))?
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (
            config,
            policy,
            command,
            working_dir,
            extra_env,
            timeout,
            explicitly_requested,
        );
        anyhow::bail!("TinyBox module support is disabled; native jail execution is unavailable")
    }
}

/// Run a host-authorized command through TinyBox's supervised passthrough
/// process lifecycle. This is used by cron, whose existing execution surface
/// runs on the host rather than inside a jail.
pub(crate) async fn execute_passthrough(
    config: &Config,
    command: &str,
    working_dir: &Path,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    #[cfg(feature = "modules")]
    {
        let config = config.clone();
        let command = command.to_owned();
        let working_dir = working_dir.to_path_buf();
        tokio::spawn(async move {
            execute_passthrough_owned(&config, &command, &working_dir, timeout).await
        })
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox passthrough worker did not finish"))?
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (config, command, working_dir, timeout);
        anyhow::bail!("TinyBox module support is disabled; command execution is unavailable")
    }
}

#[cfg(feature = "modules")]
async fn execute_passthrough_owned(
    config: &Config,
    command: &str,
    working_dir: &Path,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    let resource = tinybox::reserve(config, contract::ReserveRequest::Resource)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not reserve a command resource"))?;
    let result = async {
        let workspace = path_string(working_dir, "command working directory")?;
        tinybox::create(
            config,
            contract::CreateRequest {
                resource: resource.clone(),
                backend: "passthrough".to_owned(),
                host: contract::HostConfig::Local,
                workspace: contract::Workspace::Directory(workspace),
                // Cron's existing host command inherits network access; this
                // passthrough resource does not add a sandbox restriction.
                network: contract::NetworkPolicy::Open,
                resources: contract::ResourceLimits::default(),
                ports: Vec::new(),
                env: BTreeMap::new(),
            },
        )
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not create a command resource"))?;

        let request = shell_request(&resource, command, working_dir, BTreeMap::new(), true)?;
        let process = tinybox::reserve(config, contract::ReserveRequest::Process(resource.clone()))
            .await
            .map_err(|_| anyhow::anyhow!("TinyBox could not reserve a command process"))?;
        let process = tinybox::start_exec(
            config,
            contract::SpawnRequest {
                process,
                command: request,
            },
        )
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not start a command process"))?;
        collect_process(config, process, timeout).await
    }
    .await;
    let close = tinybox::close(config, resource).await;
    match (result, close) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(_)) => anyhow::bail!("TinyBox did not acknowledge command cleanup"),
        (Err(error), Err(_)) => Err(error),
    }
}

#[cfg(feature = "modules")]
async fn execute_owned(
    config: &Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    explicitly_requested: bool,
) -> anyhow::Result<SandboxExecResult> {
    let status = tinybox::jail_status(config)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail status is unavailable"))?;
    if !status.available || !status.suitable_for_untrusted_code {
        anyhow::bail!("TinyBox native jail cannot meet the required isolation floor");
    }
    if explicitly_requested && status.backend != "landlock" {
        anyhow::bail!("requested Landlock sandbox is unavailable");
    }

    let resource = tinybox::reserve(config, contract::ReserveRequest::Resource)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not reserve a native jail resource"))?;
    let result = async {
        let workspace = path_string(&policy.workspace_root, "sandbox workspace")?;
        tinybox::create(
            config,
            contract::CreateRequest {
                resource: resource.clone(),
                backend: "passthrough".to_owned(),
                host: contract::HostConfig::Local,
                workspace: contract::Workspace::Directory(workspace),
                network: contract::NetworkPolicy::default(),
                resources: contract::ResourceLimits::default(),
                ports: Vec::new(),
                env: BTreeMap::new(),
            },
        )
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not create a native jail resource"))?;

        let scratch = super::ops::CallDir::create(
            super::ops::sandbox_scratch_root(&policy.state_dir),
            "scratch",
        )?;
        let (env, read_write) = execution_environment(policy, extra_env, &scratch.path)?;
        let exec = shell_request(&resource, command, working_dir, env, false)?;
        let jail = jail_request(policy, read_write)?;
        let process = tinybox::reserve(config, contract::ReserveRequest::Process(resource.clone()))
            .await
            .map_err(|_| anyhow::anyhow!("TinyBox could not reserve a native jail process"))?;
        let process = tinybox::start_jailed_exec(
            config,
            contract::SpawnRequest {
                process,
                command: exec,
            },
            jail,
        )
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not start the native jailed process"))?;

        collect_process(config, process, timeout).await
    }
    .await;
    let close = tinybox::close(config, resource).await;
    match (result, close) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(_)) => anyhow::bail!("TinyBox did not acknowledge native jail cleanup"),
        (Err(error), Err(_)) => Err(error),
    }
}

#[cfg(feature = "modules")]
fn execution_environment(
    policy: &SandboxPolicy,
    extra_env: &HashMap<OsString, OsString>,
    scratch: &Path,
) -> anyhow::Result<(BTreeMap<String, String>, Vec<String>)> {
    let mut env = BTreeMap::new();
    for name in super::ops::SANDBOX_ENV_PASSTHROUGH {
        if let Some(value) = std::env::var_os(name) {
            env.insert(
                (*name).to_owned(),
                value
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("sandbox environment is not UTF-8"))?,
            );
        }
    }
    for (name, value) in extra_env {
        let name = name
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("sandbox environment name is not UTF-8"))?;
        let value = value
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("sandbox environment value is not UTF-8"))?;
        env.insert(name.to_owned(), value.to_owned());
    }

    let scratch = path_string(scratch, "sandbox scratch directory")?;
    let read_write = policy
        .read_write_mounts
        .iter()
        .map(|path| path_string(path, "read-write grant"))
        .chain(std::iter::once(Ok(scratch.clone())))
        .collect::<anyhow::Result<Vec<_>>>()?;
    if !extra_env.contains_key(OsStr::new("TMPDIR")) {
        env.insert("TMPDIR".to_owned(), scratch.clone());
    }
    if !extra_env.contains_key(OsStr::new("TEMP")) {
        env.insert("TEMP".to_owned(), scratch.clone());
    }
    if !extra_env.contains_key(OsStr::new("TMP")) {
        env.insert("TMP".to_owned(), scratch);
    }
    Ok((env, read_write))
}

#[cfg(feature = "modules")]
fn shell_request(
    resource: &contract::ResourceId,
    command: &str,
    working_dir: &Path,
    env: BTreeMap<String, String>,
    preserve_pipe_status: bool,
) -> anyhow::Result<contract::ExecRequest> {
    let shell = if preserve_pipe_status {
        platform_shell::build_std_command_preserving_pipe_status(command)
    } else {
        platform_shell::build_std_command(command)
    };
    let program = shell
        .get_program()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("platform shell name is not UTF-8"))?;
    let mut argv = vec![program.to_owned()];
    argv.extend(
        shell
            .get_args()
            .map(|arg| {
                arg.to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow::anyhow!("platform shell argument is not UTF-8"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
    );
    Ok(contract::ExecRequest {
        resource: resource.clone(),
        argv,
        cwd: Some(path_string(working_dir, "sandbox working directory")?),
        env,
        stdin: None,
    })
}

#[cfg(feature = "modules")]
fn jail_request(
    policy: &SandboxPolicy,
    read_write: Vec<String>,
) -> anyhow::Result<contract::NativeJailConfig> {
    Ok(contract::NativeJailConfig {
        read_only: policy
            .read_only_mounts
            .iter()
            .map(|path| path_string(path, "read-only grant"))
            .collect::<anyhow::Result<Vec<_>>>()?,
        read_write,
        allow_net: policy.allow_network,
        allow_subprocess: true,
        enforcement: contract::JailEnforcement::Enforced,
        require_process_isolation: true,
    })
}

#[cfg(feature = "modules")]
async fn collect_process(
    config: &Config,
    process: contract::ProcessRef,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    use contract::{ExecutionFailure, ExecutionState, OutputStream};

    let deadline = tokio::time::Instant::now() + timeout;
    let mut cursor = 0;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut requested_cancel = false;
    let timed_out = loop {
        if !requested_cancel && tokio::time::Instant::now() >= deadline {
            tinybox::cancel(config, process.clone())
                .await
                .map_err(|_| {
                    anyhow::anyhow!("TinyBox could not acknowledge process cancellation")
                })?;
            requested_cancel = true;
        }
        let batch = tinybox::read_output(config, process.clone(), cursor)
            .await
            .map_err(|_| anyhow::anyhow!("TinyBox could not read native jail output"))?;
        if !batch.chunks.is_empty() && batch.next_sequence <= cursor {
            anyhow::bail!("TinyBox returned an invalid native jail output cursor");
        }
        let has_chunks = !batch.chunks.is_empty();
        let state = batch.state;
        for chunk in batch.chunks {
            match chunk.stream {
                OutputStream::Stdout => stdout.extend_from_slice(&chunk.bytes),
                OutputStream::Stderr => stderr.extend_from_slice(&chunk.bytes),
            }
        }
        cursor = batch.next_sequence;
        match state {
            ExecutionState::Running => tokio::time::sleep(Duration::from_millis(20)).await,
            ExecutionState::Exited { .. } if has_chunks => continue,
            ExecutionState::Exited { exit_code } => {
                tinybox::release_output(config, process)
                    .await
                    .map_err(|_| anyhow::anyhow!("TinyBox could not release native jail output"))?;
                return Ok(SandboxExecResult {
                    exit_code,
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    timed_out: false,
                });
            }
            ExecutionState::Cancelled if has_chunks => continue,
            ExecutionState::Cancelled => break requested_cancel,
            ExecutionState::Failed {
                code: ExecutionFailure::CleanupFailed,
            } => anyhow::bail!("TinyBox retained native process cleanup for retry"),
            ExecutionState::Failed { .. } => {
                anyhow::bail!("TinyBox native jailed execution failed")
            }
        }
    };
    tinybox::release_output(config, process)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not release native jail output"))?;
    Ok(SandboxExecResult {
        exit_code: -1,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: if timed_out {
            format!("Command timed out after {}s", timeout.as_secs())
        } else {
            String::from_utf8_lossy(&stderr).into_owned()
        },
        timed_out,
    })
}

#[cfg(feature = "modules")]
fn path_string(path: &Path, label: &str) -> anyhow::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("{label} path is not UTF-8"))
}

#[cfg(test)]
#[path = "native_jail_tests.rs"]
mod tests;
