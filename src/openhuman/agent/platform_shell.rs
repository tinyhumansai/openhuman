//! Cross-platform shell selection for spawning agent shell commands.
//!
//! Consolidates the "which shell binary do we spawn?" decision so
//! [`NativeRuntime::build_shell_command`](super::host_runtime::NativeRuntime)
//! and the sandbox execution paths in
//! [`crate::openhuman::sandbox::ops`] can share one Windows-aware
//! implementation. Prior to this module the sandbox paths hardcoded
//! `Command::new("sh")`, which fails at `CreateProcessW` on Windows in
//! ~30ms because `sh` is not in `PATH` (#4705).
//!
//! **Shell choice per platform:**
//!
//! - **Windows** → `cmd.exe /C <command>`. Chosen over PowerShell because
//!   Windows users expect `%VAR%` expansion (`echo %USERPROFILE%`) and
//!   byte-transparent `>` / `2>` redirection for the sandboxed output-
//!   capture path in
//!   [`crate::openhuman::sandbox::ops::execute_local_jail`]. PowerShell
//!   5.1's `>` writes UTF-16LE and does not expand `%VAR%`.
//! - **Unix** → `bash -lc "set -o pipefail\n<command>"` when bash is
//!   available at `/usr/bin/bash` or `/bin/bash`, otherwise `sh -lc
//!   <command>`. `set -o pipefail` surfaces a failed stage in a pipeline
//!   (e.g. `pip install … | tail`) as a non-zero exit instead of being
//!   masked by the last stage — without it the harness records the call
//!   as successful and the repeated-failure circuit breaker
//!   (`RepeatedToolFailureMiddleware`) never trips, so the agent loops
//!   on a silently-failing command. `/bin/sh` is dash on Debian/Ubuntu
//!   and rejects `set -o pipefail`, so this is gated on bash actually
//!   being present; otherwise we fall back to plain sh.

use std::path::Path;

/// Build a [`tokio::process::Command`] that runs `command` under the
/// platform's default shell. Callers are responsible for setting
/// `current_dir`, environment, and stdio.
pub fn build_tokio_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(shell_program());
    // `as_std_mut()` so the Windows arm can reach `raw_arg` (only defined on
    // `std::process::Command`); the tokio wrapper forwards the raw arg.
    configure_shell_args(cmd.as_std_mut(), command);
    cmd
}

/// [`std::process::Command`] variant for callers that hand the command
/// to [`crate::openhuman::cwd_jail::spawn`], which is built around
/// `std::process::Command` (not the tokio variant).
pub fn build_std_command(command: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(shell_program());
    configure_shell_args(&mut cmd, command);
    cmd
}

/// Shell binary for the current platform. Single source of truth shared by
/// [`build_tokio_command`] and [`build_std_command`] — future changes to the
/// platform matrix (adding pwsh, changing pipefail semantics) belong here plus
/// [`configure_shell_args`], so both `Command` flavours stay in lockstep.
fn shell_program() -> &'static str {
    if cfg!(windows) {
        "cmd"
    } else {
        bash_path().unwrap_or("sh")
    }
}

/// Append the shell flag + command payload to `cmd`.
///
/// On Windows the payload MUST go through `raw_arg`, not `arg`: Rust's `arg`
/// applies MSVCRT (`CommandLineToArgvW`) quoting, escaping any interior `"` as
/// `\"`. But `cmd.exe` does not understand `\"` — it only toggles quote state
/// on a bare `"`. Handed to `cmd /C` via `arg`, the `>` / `2>` operators in a
/// redirect wrap (see [`wrap_with_output_redirection`]) land inside a cmd
/// quote-span, so no redirection happens and the `.sandbox_stdout` /
/// `.sandbox_stderr` capture files are never written. `raw_arg` passes the
/// string to cmd verbatim, which is exactly the byte-transparent contract this
/// module promises. `/C` itself has no special characters.
#[cfg(windows)]
fn configure_shell_args(cmd: &mut std::process::Command, command: &str) {
    use std::os::windows::process::CommandExt;
    cmd.arg("/C").raw_arg(command);
}

/// Unix arm: `bash -lc "set -o pipefail\n<command>"` when bash is present
/// (so a masked pipe-stage failure still surfaces), else plain `sh -lc`.
#[cfg(not(windows))]
fn configure_shell_args(cmd: &mut std::process::Command, command: &str) {
    if bash_path().is_some() {
        cmd.arg("-lc").arg(format!("set -o pipefail\n{command}"));
    } else {
        cmd.arg("-lc").arg(command);
    }
}

/// Wrap `command` so that stdout and stderr redirect to the given file
/// paths, using shell syntax compatible with the platform's default
/// shell as selected by [`build_tokio_command`] / [`build_std_command`].
///
/// Used by [`crate::openhuman::sandbox::ops::execute_local_jail`] to
/// capture output on backends (macOS Seatbelt) that rebuild the command
/// internally and don't forward piped stdio settings.
pub fn wrap_with_output_redirection(
    command: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) -> String {
    if cfg!(windows) {
        // cmd.exe has no `{ … }` command grouping, but `>`/`2>` bind to
        // the whole /C payload when placed at the end, so a plain
        // trailing redirect captures the full output for both single
        // commands and pipelines. Double-quote paths so backslashes,
        // spaces, and `(` / `)` inside typical Windows workspace paths
        // (e.g. `C:\Program Files (x86)\…`) don't break parsing.
        format!(
            "{command} > \"{}\" 2> \"{}\"",
            stdout_path.display(),
            stderr_path.display()
        )
    } else {
        // sh/bash need `{ … ; }` grouping so a semicolon- or pipe-
        // separated multi-stage `command` routes *all* stages' output
        // to the temp files. Without the group `a; b > out` would only
        // redirect `b`. Single-quote paths so shell metacharacters in
        // the workspace path stay literal.
        format!(
            "{{ {command} ; }} > '{}' 2> '{}'",
            stdout_path.display(),
            stderr_path.display()
        )
    }
}

/// Locate a `bash` binary once (cached — hit on every shell call) for
/// the `pipefail` wrapper. Returns `None` on hosts without bash at a
/// standard path (Windows, minimal containers), where we fall back to
/// plain `sh` without pipefail. Exposed `pub(crate)` so regression
/// tests in [`super::host_runtime`] can skip the pipefail assertions
/// on bash-less hosts.
pub(crate) fn bash_path() -> Option<&'static str> {
    static BASH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BASH.get_or_init(|| {
        ["/usr/bin/bash", "/bin/bash"]
            .into_iter()
            .find(|p| Path::new(p).exists())
            .map(str::to_string)
    })
    .as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn tokio_command_selects_platform_shell() {
        let cmd = build_tokio_command("echo hi");
        let prog = cmd.as_std().get_program().to_string_lossy().into_owned();
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        if cfg!(windows) {
            assert_eq!(prog, "cmd");
            assert_eq!(args, vec!["/C".to_string(), "echo hi".to_string()]);
        } else if let Some(bash) = bash_path() {
            assert_eq!(prog, bash);
            assert_eq!(
                args,
                vec!["-lc".to_string(), "set -o pipefail\necho hi".to_string()]
            );
        } else {
            assert_eq!(prog, "sh");
            assert_eq!(args, vec!["-lc".to_string(), "echo hi".to_string()]);
        }
    }

    #[test]
    fn std_command_selects_platform_shell() {
        let cmd = build_std_command("echo hi");
        let prog = cmd.get_program().to_string_lossy().into_owned();
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        if cfg!(windows) {
            assert_eq!(prog, "cmd");
            assert_eq!(args, vec!["/C".to_string(), "echo hi".to_string()]);
        } else if let Some(bash) = bash_path() {
            assert_eq!(prog, bash);
            assert_eq!(
                args,
                vec!["-lc".to_string(), "set -o pipefail\necho hi".to_string()]
            );
        } else {
            assert_eq!(prog, "sh");
            assert_eq!(args, vec!["-lc".to_string(), "echo hi".to_string()]);
        }
    }

    #[test]
    fn output_redirection_wraps_per_platform() {
        let stdout = PathBuf::from("/tmp/openhuman/out.log");
        let stderr = PathBuf::from("/tmp/openhuman/err.log");
        let wrapped = wrap_with_output_redirection("echo hi", &stdout, &stderr);

        if cfg!(windows) {
            assert_eq!(
                wrapped,
                r#"echo hi > "/tmp/openhuman/out.log" 2> "/tmp/openhuman/err.log""#
            );
        } else {
            assert_eq!(
                wrapped,
                "{ echo hi ; } > '/tmp/openhuman/out.log' 2> '/tmp/openhuman/err.log'"
            );
        }
    }
}
