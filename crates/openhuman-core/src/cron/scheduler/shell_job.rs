//! Execution of `JobType::Shell` jobs under the security policy, with a
//! forbidden-path argument scan and a hard timeout.

use crate::config::Config;
use crate::security::policy::{CommandFlow, CommandQuery, CommandStage, Refusal};
use crate::security::SecurityPolicy;
use tinyflows_schedule::types::CronJob;
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
    security
        .check_command(CommandQuery {
            command,
            declared: None,
            flow: CommandFlow::Scheduled,
            stage,
        })
        .await
        .map(|_| ())
        .map_err(|refusal| {
            log::debug!("[cron][shell] command refused stage={stage:?} reason={refusal:?}");
            match refusal {
                Refusal::Protected => "blocked by security policy: command rejected".to_owned(),
                Refusal::RateLimit => "blocked by security policy: rate limit exceeded".to_owned(),
                Refusal::Unavailable => "Command analysis is unavailable.".to_owned(),
                _ => "blocked by security policy: command not allowed".to_owned(),
            }
        })
}

#[cfg(test)]
pub(super) fn is_env_assignment(word: &str) -> bool {
    word.contains('=')
        && word
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

#[cfg(test)]
pub(super) fn strip_wrapping_quotes(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

#[cfg(test)]
pub(super) fn forbidden_path_argument(security: &SecurityPolicy, command: &str) -> Option<String> {
    let mut normalized = command.to_string();
    for sep in ["&&", "||"] {
        normalized = normalized.replace(sep, "\x00");
    }
    for sep in ['\n', ';', '|'] {
        normalized = normalized.replace(sep, "\x00");
    }

    for segment in normalized.split('\x00') {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        // Skip leading env assignments and executable token.
        let mut idx = 0;
        while idx < tokens.len() && is_env_assignment(tokens[idx]) {
            idx += 1;
        }
        if idx >= tokens.len() {
            continue;
        }
        idx += 1;

        for token in &tokens[idx..] {
            let candidate = strip_wrapping_quotes(token);
            if candidate.is_empty() || candidate.starts_with('-') || candidate.contains("://") {
                continue;
            }

            let looks_like_path = candidate.starts_with('/')
                || candidate.starts_with("./")
                || candidate.starts_with("../")
                || candidate.starts_with("~/")
                || candidate.contains('/');

            if looks_like_path && !security.is_path_string_allowed(candidate) {
                return Some(candidate.to_string());
            }
        }
    }

    None
}

pub(super) async fn run_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    run_job_command_with_timeout(
        config,
        security,
        job,
        Duration::from_secs(SHELL_JOB_TIMEOUT_SECS),
    )
    .await
}

pub(super) async fn run_job_command_with_timeout(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
    timeout: Duration,
) -> (bool, String) {
    if !security.can_act() {
        return (
            false,
            "blocked by security policy: autonomy is read-only".to_string(),
        );
    }

    if security.is_rate_limited() {
        return (
            false,
            "blocked by security policy: rate limit exceeded".to_string(),
        );
    }

    if let Err(reason) = admit_shell_command(security, &job.command).await {
        return (false, reason);
    }

    if !security.record_action() {
        return (
            false,
            "blocked by security policy: action budget exhausted".to_string(),
        );
    }

    // Cron keeps host execution semantics (the host's own network, no jail)
    // and a pipeline's default exit status: retries are spent on it.
    let output = match crate::sandbox::shell_exec::run_host(
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
        Err(_) => return (false, "Command execution is unavailable.".to_owned()),
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
