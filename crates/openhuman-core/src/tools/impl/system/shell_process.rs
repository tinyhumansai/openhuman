//! `shell_process`: follow a shell command the turn left running.
//!
//! A `shell` call whose command is still running at its yield point returns
//! a process id instead of waiting; this tool waits on that process for more
//! output or its end, stops it, or reports its status
//! (`sandbox::shell_processes`). A wait blocks on the host, so following a long
//! command takes a few calls, not a polling loop.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext, ToolTimeout};

use super::shell_support::{command_failure, human_duration, printed, tool_timeout_message};
use crate::sandbox::shell_processes::{Ended, Report, ReportState, QUIET_WAITS_NOTE};

/// How long a wait blocks when the call does not say.
const DEFAULT_WAIT_SECS: u64 = 120;
/// The longest one wait may block.
const MAX_WAIT_SECS: u64 = 600;

/// Follows a shell command left running past its yield point.
#[derive(Default)]
pub struct ShellProcessTool;

impl ShellProcessTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ShellProcessTool {
    fn name(&self) -> &str {
        "shell_process"
    }

    fn description(&self) -> &str {
        "Follow a shell command that is still running: `shell` returns a process id (`p1`) \
         instead of waiting when a command outlives its yield point. `wait` (the default) blocks \
         until the process writes and ends or `wait_secs` pass, and returns only the output \
         written since the last call, then its exit code or that it is still running. `kill` \
         stops it and returns the rest of its output. `status` says whether it is still running. \
         Processes stop at their own timeout and when the turn ends."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "process": {
                    "type": "string",
                    "description": "The id `shell` returned, such as `p1`."
                },
                "action": {
                    "type": "string",
                    "enum": ["wait", "kill", "status"],
                    "description": "`wait` (default) for more output or the end, `kill` to stop it, `status` to check it."
                },
                "wait_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_WAIT_SECS,
                    "description": "For `wait`: the longest to block, default 120."
                }
            },
            "required": ["process"]
        })
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        Some(30_000)
    }

    /// A wait blocks for its own `wait_secs`; the harness's per-call timeout
    /// must not cut it short.
    fn timeout_policy(&self, _args: &serde_json::Value) -> ToolTimeout {
        ToolTimeout::Unbounded
    }

    /// Reading or stopping a command the turn already started (and that was
    /// already approved) has no effect of its own.
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let Some(id) = args.get("process").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error(
                "Missing 'process': the id `shell` returned, such as `p1`.",
            ));
        };
        let Some(processes) =
            crate::agent::tinyagents::host::run_context::shell_processes_for_tool(context)
        else {
            return Ok(ToolResult::error(
                "No running processes here: only a turn can leave a shell command running.",
            ));
        };
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("wait");
        let report = match action {
            "wait" => {
                let secs = args
                    .get("wait_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_WAIT_SECS)
                    .clamp(1, MAX_WAIT_SECS);
                tracing::debug!(process = %id, wait_secs = secs, "[shell_process] wait");
                processes.wait(id, Duration::from_secs(secs)).await
            }
            "kill" => processes.kill(id).await,
            "status" => processes.status(id).await,
            other => {
                return Ok(ToolResult::error(format!(
                    "Unknown action `{other}`: use `wait`, `kill` or `status`."
                )))
            }
        };
        Ok(match report {
            Ok(report) => render(&report, action),
            Err(message) => ToolResult::error(message),
        })
    }
}

/// The tool result for one report. A process that ended on its own reads like
/// a finished `shell` call: an exit report for a non-zero code, the timeout
/// line when its deadline stopped it.
pub(super) fn render(report: &Report, action: &str) -> ToolResult {
    let id = &report.id;
    let elapsed = human_duration(report.elapsed);
    match &report.state {
        ReportState::Running if action == "status" => ToolResult::success(format!(
            "Process {id} is still running ({elapsed}; it stops at its {} timeout).",
            human_duration(report.timeout)
        )),
        ReportState::Running if action == "kill" => ToolResult::error(format!(
            "Process {id} did not stop: it is still running ({elapsed}) after the stop request. \
             Check it with {{\"process\": \"{id}\", \"action\": \"status\"}}; it is stopped \
             at its {} timeout or when the turn ends.",
            human_duration(report.timeout)
        )),
        ReportState::Ended(ended) if action == "status" => ToolResult::success(format!(
            "Process {id} has {} ({elapsed}); {{\"process\": \"{id}\"}} returns its last output.",
            ended_phrase(*ended)
        )),
        ReportState::Running => {
            let mut out = format!("Process {id} is still running ({elapsed}).");
            out.push_str(&printed(
                &report.stdout,
                &report.stderr,
                "New output:",
                "No new output.",
            ));
            if report.quiet_waits >= QUIET_WAITS_NOTE {
                out.push_str(&format!(
                    "\nNothing new in the last {} waits: if it is stuck or no longer needed, \
                     stop it with {{\"process\": \"{id}\", \"action\": \"kill\"}}.",
                    report.quiet_waits
                ));
            }
            ToolResult::success(out)
        }
        ReportState::Ended(Ended::Exited(0)) => {
            let mut out = format!("Process {id} exited with code 0 after {elapsed}.");
            out.push_str(&printed(
                &report.stdout,
                &report.stderr,
                "Final output:",
                "No further output.",
            ));
            ToolResult::success(out)
        }
        ReportState::Ended(Ended::Exited(code)) => {
            command_failure(Some(*code), &report.stdout, &report.stderr)
        }
        ReportState::Ended(Ended::TimedOut) => {
            let mut out = tool_timeout_message(report.timeout.as_secs());
            out.push_str(&printed(
                &report.stdout,
                &report.stderr,
                "Output before the kill:",
                "No further output before the kill.",
            ));
            ToolResult::error(out)
        }
        ReportState::Ended(Ended::Stopped) => {
            let mut out = format!("Process {id} stopped after {elapsed}.");
            out.push_str(&printed(
                &report.stdout,
                &report.stderr,
                "Its last output:",
                "No further output.",
            ));
            ToolResult::success(out)
        }
        ReportState::AlreadyEnded(ended) => ToolResult::success(format!(
            "Process {id} already {}; its output was returned earlier.",
            ended_phrase(*ended)
        )),
    }
}

/// How a process ended, as a verb phrase.
fn ended_phrase(ended: Ended) -> String {
    match ended {
        Ended::Exited(code) => format!("exited with code {code}"),
        Ended::TimedOut => "stopped at its timeout".to_owned(),
        Ended::Stopped => "stopped".to_owned(),
    }
}

#[cfg(test)]
#[path = "shell_process_tests.rs"]
mod tests;
