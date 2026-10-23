//! Where `command` hooks run: behind the untrusted-spawn gate (issue #7328,
//! phase 5).
//!
//! Hook commands are text from `hooks.json` layers (system, user and the
//! project's own file), so they are not host-authored argv. The hook engine
//! (`tinyagents_runtime::command_hooks`) hands each one to this
//! [`CommandSpawner`], which runs it through [`crate::sandbox::untrusted::run`]:
//! the TinyBox native jail (or the configured Docker container), refused when
//! the isolation floor cannot be met. A refusal is an `Err` to the engine, which
//! turns it into a denial for a `fail_closed` hook and a logged no-op otherwise.
//! There is no unjailed fallback here; the operator's own sandbox opt-out is
//! handled inside the gate and is never honoured under SaaS.
//!
//! The jail picks the shell, so nothing here selects one.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tinyagents_runtime::command_hooks::{CommandOutput, CommandRequest, CommandSpawner};

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::sandbox::untrusted::{self, UntrustedShell};

/// Spawns hook commands under the untrusted-spawn gate.
pub(super) struct SandboxedHookSpawner;

#[async_trait]
impl CommandSpawner for SandboxedHookSpawner {
    async fn run(&self, request: CommandRequest) -> Result<CommandOutput, String> {
        let config = DispatchCtx::ambient()
            .load_config()
            .await
            .map_err(|error| {
                log::warn!("[hooks] refused: cannot read config for the sandbox: {error}");
                format!("cannot read config to sandbox the hook: {error}")
            })?;
        run_in(&config, request).await
    }
}

/// Run `request` under the gate with `config` as the scope's configuration.
async fn run_in(config: &Config, request: CommandRequest) -> Result<CommandOutput, String> {
    let CommandRequest {
        command,
        cwd,
        env,
        stdin,
        timeout,
    } = request;
    let cwd: PathBuf = cwd.unwrap_or_else(|| config.action_dir.clone());
    log::debug!(
        "[hooks] spawning a command hook under the untrusted gate timeout_ms={} env_vars={} stdin_bytes={}",
        timeout.as_millis(),
        env.len(),
        stdin.len()
    );
    let result = untrusted::run(UntrustedShell {
        dispatch_ctx: None,
        config,
        command: &command,
        cwd: &cwd,
        state_dir: &config.workspace_dir,
        timeout,
        preserve_pipeline_status: false,
        env: &env,
        stdin: Some(&stdin),
    })
    .await;
    outcome(result, timeout)
}

fn outcome(
    result: anyhow::Result<crate::sandbox::types::SandboxExecResult>,
    timeout: Duration,
) -> Result<CommandOutput, String> {
    match result {
        Ok(output) if output.timed_out => Err(format!("timed out after {}s", timeout.as_secs())),
        Ok(output) => Ok(CommandOutput {
            exit_code: (output.exit_code >= 0).then_some(output.exit_code),
            stdout: output.stdout.into_bytes(),
            stderr: output.stderr.into_bytes(),
        }),
        Err(error) if error.is::<crate::sandbox::ops::IsolationFloor>() => {
            log::warn!("[hooks] refused: the sandbox is below the isolation floor; the hook command was not started");
            Err(format!("blocked by security policy: {error}"))
        }
        Err(error) => {
            log::warn!("[hooks] sandboxed hook execution failed: {error}");
            Err(format!("sandboxed execution failed: {error}"))
        }
    }
}

#[cfg(test)]
#[path = "spawner_tests.rs"]
mod tests;
