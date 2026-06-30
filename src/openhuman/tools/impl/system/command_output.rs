//! Shared output formatting for the shell-family command tools (`shell`,
//! `node_exec`, `npm_exec`).
//!
//! # Why this exists (code_executor no-progress loop, #4095)
//!
//! Each shell-family tool previously formatted a FAILED command as
//! `if stderr.is_empty() { stdout } else { stderr }` with no exit code. That
//! lost the two signals the agent most needs to recover:
//!
//!   1. **stdout was dropped whenever stderr was non-empty.** Compilers, test
//!      runners and linters routinely write diagnostics to stdout; the model
//!      saw only stderr and lost half the failure context.
//!   2. **the exit code was never surfaced.** The model could not tell a `127`
//!      (command / dependency not found) or a `126` (permission denied — often a
//!      sandbox restriction) from a generic `1`, so it could not recognise an
//!      un-retryable wall and re-ran the identical command. The harness
//!      repeated-failure circuit breaker (`RepeatFailureGuard`, see
//!      `agent/harness/tool_loop.rs`) still bounds that loop, but only after a
//!      few wasted iterations and with a generic halt message, because the
//!      root-cause signal had already been thrown away.
//!
//! This module surfaces the exit code AND both streams on failure, and appends a
//! short hint for the well-known dependency/sandbox exit codes so the agent can
//! adapt (install/declare the dependency, request escalation, or report the
//! blocker) instead of retrying blindly. The success path is intentionally left
//! as raw stdout so existing callers that parse a command's output (e.g. `pwd`)
//! keep working unchanged.

use crate::openhuman::tools::traits::ToolResult;

/// Hint appended after the exit code for the exit statuses that almost always
/// mean "this exact command cannot succeed on retry here". Empty for every other
/// code so an ordinary application failure is never editorialised.
fn exit_code_hint(code: i32) -> &'static str {
    match code {
        127 => {
            " — command not found: a required executable or dependency is \
                 missing or not on PATH. Install/declare it, use an available \
                 alternative, or report the blocker — do NOT re-run the same command"
        }
        126 => {
            " — permission denied or not executable: often a sandbox \
                 restriction. This will not succeed on retry — report the blocker \
                 or request escalation instead of repeating the command"
        }
        _ => "",
    }
}

/// Render a finished command's exit status + captured streams into the text the
/// model sees on FAILURE. Never drops a non-empty stream; always states the exit
/// code (or that the process was terminated by a signal).
pub(crate) fn render_command_failure(exit_code: Option<i32>, stdout: &str, stderr: &str) -> String {
    let mut out = match exit_code {
        Some(code) => format!("Command failed (exit code {code}{})", exit_code_hint(code)),
        None => "Command failed (terminated by a signal — no exit code)".to_string(),
    };
    let stdout = stdout.trim_end();
    let stderr = stderr.trim_end();
    if !stdout.is_empty() {
        out.push_str("\n[stdout]\n");
        out.push_str(stdout);
    }
    if !stderr.is_empty() {
        out.push_str("\n[stderr]\n");
        out.push_str(stderr);
    }
    if stdout.is_empty() && stderr.is_empty() {
        out.push_str("\n(no output was captured on stdout or stderr)");
    }
    out
}

/// A `ToolResult::error` carrying [`render_command_failure`]. The single failure
/// constructor shared by every shell-family tool, on both the native and the
/// sandboxed execution path, so the surfaced shape can't drift between them.
pub(crate) fn command_failure(exit_code: Option<i32>, stdout: &str, stderr: &str) -> ToolResult {
    ToolResult::error(render_command_failure(exit_code, stdout, stderr))
}

/// Normalise a sandbox backend's `exit_code` into the `Option<i32>` the
/// formatter expects. The sandbox layer uses `-1` as its sentinel for
/// "terminated / no real exit code" (see `sandbox::types::SandboxExecResult`);
/// map any negative value to `None` so it renders as a signal termination rather
/// than the literal `exit code -1`.
pub(crate) fn sandbox_exit_code(code: i32) -> Option<i32> {
    if code < 0 {
        None
    } else {
        Some(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_surfaces_exit_code_and_both_streams() {
        let rendered = render_command_failure(Some(7), "the-stdout-line", "the-stderr-line");
        assert!(
            rendered.contains("exit code 7"),
            "exit code missing: {rendered}"
        );
        assert!(
            rendered.contains("the-stdout-line"),
            "stdout dropped on failure: {rendered}"
        );
        assert!(
            rendered.contains("the-stderr-line"),
            "stderr dropped on failure: {rendered}"
        );
    }

    #[test]
    fn failure_keeps_stdout_even_when_stderr_present() {
        // The exact regression: the old `if stderr.is_empty() { stdout } else
        // { stderr }` formatting threw stdout away whenever stderr existed.
        let rendered = render_command_failure(Some(1), "diagnostic-on-stdout", "error-on-stderr");
        assert!(rendered.contains("diagnostic-on-stdout"));
        assert!(rendered.contains("error-on-stderr"));
    }

    #[test]
    fn exit_127_hints_missing_command_or_dependency() {
        let rendered = render_command_failure(Some(127), "", "pytest: command not found");
        assert!(rendered.contains("exit code 127"));
        assert!(
            rendered.to_lowercase().contains("command not found"),
            "127 should hint at a missing command/dependency: {rendered}"
        );
    }

    #[test]
    fn exit_126_hints_permission_or_sandbox() {
        let rendered = render_command_failure(Some(126), "", "permission denied");
        assert!(rendered.contains("exit code 126"));
        assert!(
            rendered.to_lowercase().contains("sandbox")
                || rendered.to_lowercase().contains("permission denied"),
            "126 should hint at a permission/sandbox wall: {rendered}"
        );
    }

    #[test]
    fn ordinary_failure_code_gets_no_hint() {
        let rendered = render_command_failure(Some(1), "", "boom");
        // No editorialising for a generic application failure.
        assert!(rendered.contains("exit code 1"));
        assert!(!rendered.contains("command not found"));
        assert!(!rendered.contains("sandbox"));
    }

    #[test]
    fn signal_termination_has_no_exit_code() {
        let rendered = render_command_failure(None, "", "");
        assert!(rendered.contains("terminated by a signal"));
        assert!(rendered.contains("no output was captured"));
    }

    #[test]
    fn sandbox_negative_exit_code_maps_to_signal() {
        assert_eq!(sandbox_exit_code(-1), None);
        assert_eq!(sandbox_exit_code(0), Some(0));
        assert_eq!(sandbox_exit_code(7), Some(7));
    }
}
