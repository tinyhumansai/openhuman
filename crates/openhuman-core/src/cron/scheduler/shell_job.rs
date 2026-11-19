//! Execution of `JobType::Shell` jobs under the security policy and a hard
//! timeout (`[cron] shell_timeout_secs`, default 120 s).

use crate::config::Config;
use crate::security::SecurityPolicy;
use tinyflows_bus::schedule::types::CronJob;
use tokio::time::Duration;

/// Prefix the retry loop (`executor.rs#classify_failure`) reads as final.
pub(super) const POLICY_REFUSAL_PREFIX: &str = "blocked by security policy:";

/// Check and reserve a scheduled shell command through TinySecurity.
///
/// A refusal that waiting cannot change (the policy said no) carries
/// [`POLICY_REFUSAL_PREFIX`], so the job fails without spending its retries.
/// An unavailable module or an exhausted action budget may recover, so those
/// stay retryable.
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
        .map_err(refusal_output)?;
    if verdict.gate == crate::security::policy::GateDecision::Block {
        return Err(format!("{POLICY_REFUSAL_PREFIX} command rejected"));
    }
    Ok(())
}

pub(super) async fn run_job_command(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    let timeout: Duration = config.cron.shell_timeout();
    tracing::debug!(
        job_id = %job.id,
        timeout_secs = timeout.as_secs(),
        "[cron:shell] running scheduled shell job"
    );
    run_job_command_with_timeout(ctx, config, security, job, timeout).await
}

/// The job output for a refused command: final unless it may recover.
fn refusal_output(reason: crate::security::policy::Refusal) -> String {
    use crate::security::policy::Refusal;
    match reason {
        Refusal::Unavailable | Refusal::RateLimit => reason.to_string(),
        _ => format!("{POLICY_REFUSAL_PREFIX} {reason}"),
    }
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

#[cfg(test)]
#[path = "shell_job_tests.rs"]
mod tests;
