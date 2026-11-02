//! Execution of `JobType::Shell` jobs under the security policy and a hard
//! timeout.

use crate::config::Config;
use crate::security::SecurityPolicy;
use tinyflows_bus::schedule::types::CronJob;
use tokio::time::Duration;

pub(super) const SHELL_JOB_TIMEOUT_SECS: u64 = 120;

/// Check and reserve a scheduled shell command through TinySecurity.
pub(crate) async fn authorize_shell_command(
    _config: &Config,
    security: &SecurityPolicy,
    command: &str,
) -> Result<(), String> {
    let verdict = security
        .check_command(crate::security::policy::CommandQuery {
            command,
            declared: None,
            flow: crate::security::policy::CommandFlow::Scheduled,
            stage: crate::security::policy::CommandStage::Execute,
        })
        .await
        .map_err(|reason| reason.to_string())?;
    if verdict.gate == crate::security::policy::GateDecision::Block {
        return Err("blocked by security policy: command rejected".to_owned());
    }
    Ok(())
}

pub(super) async fn run_job_command(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    run_job_command_with_timeout(
        ctx,
        config,
        security,
        job,
        Duration::from_secs(SHELL_JOB_TIMEOUT_SECS),
    )
    .await
}

pub(super) async fn run_job_command_with_timeout(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
    timeout: Duration,
) -> (bool, String) {
    if let Err(reason) = authorize_shell_command(config, security, &job.command).await {
        return (false, reason);
    }

    // Cron keeps host execution semantics (the host's own network, no jail)
    // and a pipeline's default exit status: retries are spent on it.
    let output = match crate::sandbox::shell_exec::run_host(
        ctx,
        config,
        &job.command,
        &config.action_dir,
        &config.workspace_dir,
        std::collections::BTreeMap::new(),
        timeout,
        true,
        None,
    )
    .await
    {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(
                job_id = %job.id,
                error = %format!("{error:#}"),
                "[cron:shell] command could not run"
            );
            return (false, "Command execution is unavailable.".to_owned());
        }
    };
    if output.timed_out {
        return (
            false,
            format!("job timed out after {}s", timeout.as_secs_f64()),
        );
    }
    let combined = format!(
        "status=exit status: {}\nstdout:\n{}\nstderr:\n{}",
        output.exit_code,
        output.stdout.trim(),
        output.stderr.trim()
    );
    (output.success(), combined)
}
