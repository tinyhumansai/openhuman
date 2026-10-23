//! Execution of `JobType::Shell` jobs under the security policy (the module
//! analyses the command) and a hard timeout.

use crate::config::Config;
use crate::security::policy::{CommandFlow, CommandQuery, CommandStage, Refusal};
use crate::security::GateDecision;
use crate::security::SecurityPolicy;
use tinyflows_bus::schedule::types::CronJob;
use tokio::time::Duration;

pub(super) const SHELL_JOB_TIMEOUT_SECS: u64 = 120;

/// Check a scheduled shell command when a job is created or edited. The
/// security module classifies it under the scheduled-execution rule (curated
/// allowlist plus the configured risk restrictions); this host only reports
/// the verdict. A missing or faulted module is a refusal, never a request to
/// parse locally. Nothing is reserved from the hourly budget.
pub(crate) async fn authorize_shell_command(
    security: &SecurityPolicy,
    command: &str,
) -> Result<(), String> {
    ask_module(security, command, CommandStage::Authorize).await
}

/// Admit a scheduled shell command immediately before it runs. As
/// [`authorize_shell_command`], and an allowed command also spends one action
/// of the scope's hourly budget.
pub(crate) async fn admit_shell_command(
    security: &SecurityPolicy,
    command: &str,
) -> Result<(), String> {
    ask_module(security, command, CommandStage::Execute).await
}

async fn ask_module(
    security: &SecurityPolicy,
    command: &str,
    stage: CommandStage,
) -> Result<(), String> {
    let verdict = security
        .check_command(CommandQuery {
            command,
            declared: None,
            flow: CommandFlow::Scheduled,
            stage,
        })
        .await
        .map_err(|refusal| {
            log::debug!("[cron][shell] command refused stage={stage:?} reason={refusal:?}");
            match refusal {
                Refusal::Protected => "blocked by security policy: command rejected".to_owned(),
                Refusal::RateLimit => "blocked by security policy: rate limit exceeded".to_owned(),
                Refusal::Unavailable => "Command analysis is unavailable.".to_owned(),
                _ => "blocked by security policy: command not allowed".to_owned(),
            }
        })?;
    match verdict.gate {
        GateDecision::Allow => Ok(()),
        GateDecision::Prompt => Err(
            "blocked by security policy: scheduled commands cannot wait for interactive approval"
                .to_owned(),
        ),
        GateDecision::Block => Err("blocked by security policy: command rejected".to_owned()),
    }
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
    _ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
    timeout: Duration,
) -> (bool, String) {
    // The module's Execute-stage check owns the autonomy tier and the hourly
    // action budget; an allowed command spends one action there.
    if let Err(reason) = admit_shell_command(security, &job.command).await {
        return (false, reason);
    }

    // A scheduled command is text typed into a schedule, so it runs under the
    // untrusted-spawn gate: the TinyBox jail (or the configured container),
    // refused when the isolation floor cannot be met. The host's own network
    // stays available and a pipeline keeps its default exit status, because
    // retries are spent on it.
    let output = match crate::sandbox::untrusted::run(crate::sandbox::untrusted::UntrustedShell {
        dispatch_ctx: None,
        config,
        command: &job.command,
        cwd: &config.action_dir,
        state_dir: &config.workspace_dir,
        timeout,
        preserve_pipeline_status: true,
        env: &crate::sandbox::untrusted::NO_ENV,
        stdin: None,
    })
    .await
    {
        Ok(output) => output,
        Err(error) if error.is::<crate::sandbox::ops::IsolationFloor>() => {
            log::warn!("[cron][shell] refused: the sandbox is below the isolation floor");
            // The `blocked by security policy:` prefix halts the retry loop: a
            // missing jail does not appear by waiting.
            return (false, format!("blocked by security policy: {error}"));
        }
        Err(error) => {
            log::warn!("[cron][shell] sandboxed execution failed: {error}");
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
