//! Cross-platform shell selection for spawning agent shell commands.
//!
//! Consolidates the "which shell binary do we spawn?" decision so
//! [`NativeRuntime::build_shell_command`](super::host_runtime::NativeRuntime)
//! and command adapters can share one Windows-aware implementation. Prior to
//! this module the sandbox paths hardcoded
//! `Command::new("sh")`, which fails at `CreateProcessW` on Windows in
//! ~30ms because `sh` is not in `PATH` (#4705).
//!
//! **Shell choice per platform:**
//!
//! - **Windows** → `cmd.exe /C <command>`. Chosen over PowerShell because
//!   Windows users expect `%VAR%` expansion (`echo %USERPROFILE%`) and
//!   byte-transparent `>` / `2>` redirection for shell commands. PowerShell
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

/// Which shell family [`build_tokio_command`] / [`build_std_command`] spawn.
///
/// A value rather than a bare `cfg!(windows)` so what is *told* to the model
/// about the shell (the `shell` tool's description) can be built and tested for
/// either platform on any host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellFlavor {
    /// `cmd.exe /C <command>` (Windows).
    Cmd,
    /// `bash -lc` / `sh -lc` (everything else).
    Posix,
}

impl ShellFlavor {
    /// The flavor this host spawns.
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Cmd
        } else {
            Self::Posix
        }
    }
}

/// Environment variables a Windows child process needs before it can run at
/// all, beyond the functional allow-list each launcher already forwards.
///
/// Every sanitized spawn path in the core calls `env_clear()` and re-forwards
/// only what it names, so this list is what makes a cleared Windows environment
/// bootable. These are forwarded from the parent environment, never synthesised
/// or hard-coded: a name the parent does not have is simply not set.
///
/// Measured consequences of omitting them (Windows 11, `SystemRoot` absent from
/// an otherwise-valid child environment):
///
/// - `node.exe` aborts during startup with
///   `Assertion failed: ncrypto::CSPRNG(nullptr, 0)` (exit 134), because the
///   OS random provider cannot initialise without a system directory.
/// - `powershell.exe` exits with `Internal Windows PowerShell error. Loading
///   managed Windows PowerShell failed with error 8009001d.`
/// - `cmd.exe` leaves `%SystemRoot%`/`%TEMP%`/`%USERPROFILE%` unexpanded. It
///   does *not* repair them for its own children, so a wrapper shell cannot
///   rescue a stripped environment.
///
/// `COMSPEC` and `PATHEXT` are here because `cmd.exe` needs `PATHEXT` to resolve
/// the `.cmd`/`.bat` shims that npm, npx and the Git toolchain are installed
/// as; `TEMP`/`TMP`/`USERPROFILE`/`APPDATA`/`LOCALAPPDATA` because tooling
/// writes scratch and reads config from them; the `ProgramFiles*` trio because
/// installers and SDK locators probe them.
///
/// Single source of truth: keep every launcher's allow-list a superset of this
/// (enforced by the launcher unit tests), and add a launcher to those tests
/// rather than inventing a sixth list.
pub const WINDOWS_PROCESS_ENV_VARS: &[&str] = &[
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
];

/// Assert that a launcher's environment allow-list covers every Windows
/// process-bootstrap variable.
///
/// Each launcher guards its own list with this, so a new launcher cannot
/// silently ship a `env_clear()` that produces an unbootable Windows child.
/// The failure this prevents is not a clean error: the child aborts inside the
/// OS crypto provider (`node` → `ncrypto::CSPRNG` assertion, `powershell` →
/// `8009001d`), and the harness reports it as a mysterious exit code rather
/// than a missing environment variable.
#[track_caller]
pub fn assert_forwards_windows_bootstrap(allowlist: &[&str], launcher: &str) {
    for var in WINDOWS_PROCESS_ENV_VARS {
        assert!(
            allowlist.contains(var),
            "{launcher} clears the child environment but does not forward \
             `{var}`; a Windows child spawned without it aborts during crypto \
             init. Add it to the allow-list."
        );
    }
}

/// Add Windows bootstrap variables to a host child. Keep these out of the
/// sandbox policy because that policy is also forwarded into Linux containers.
#[cfg(windows)]
pub fn forward_windows_bootstrap_env(cmd: &mut tokio::process::Command) -> anyhow::Result<()> {
    for var in WINDOWS_PROCESS_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            if val.is_empty() {
                anyhow::bail!("Windows bootstrap environment variable {var} is empty");
            }
            cmd.env(var, val);
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn forward_windows_bootstrap_env(_cmd: &mut tokio::process::Command) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub fn forward_windows_bootstrap_env_std(cmd: &mut std::process::Command) -> anyhow::Result<()> {
    for var in WINDOWS_PROCESS_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            if val.is_empty() {
                anyhow::bail!("Windows bootstrap environment variable {var} is empty");
            }
            cmd.env(var, val);
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn forward_windows_bootstrap_env_std(_cmd: &mut std::process::Command) -> anyhow::Result<()> {
    Ok(())
}

/// Whether the Unix arm prefixes `set -o pipefail`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PipeFail {
    /// Surface a masked pipe-stage failure, so `curl … | sh` failing in the
    /// first stage is not reported as a success.
    Surface,
    /// Leave a pipeline's exit status as the shell's own default.
    ///
    /// For a caller whose exit status is a contract with something outside this
    /// process. `pipefail` makes `false | true` fail where a plain `sh -lc`
    /// succeeded, so switching an existing surface onto the `Surface` arm
    /// silently re-statuses every command with a pipeline in it.
    AsShellDefault,
}

/// Build a [`tokio::process::Command`] that runs `command` under the
/// platform's default shell. Callers are responsible for setting
/// `current_dir`, environment, and stdio.
pub fn build_tokio_command(command: &str) -> tokio::process::Command {
    build_tokio_command_with(command, PipeFail::Surface)
}

/// [`build_tokio_command`] without the `set -o pipefail` prefix.
///
/// For a caller that records its command's exit status and acts on it —
/// [`crate::cron`]'s shell jobs report `success`/`failure` per run and spend a
/// retry budget on it. Turning `pipefail` on for those would flip an existing
/// job whose command ends in a tolerated pipe stage from success to failure,
/// which is a behaviour change to somebody's schedule rather than a lint.
///
/// Everything else about the platform matrix is shared with
/// [`build_tokio_command`], so a caller here still gets `cmd /C` on Windows
/// rather than a shell that does not exist there.
pub fn build_tokio_command_preserving_pipe_status(command: &str) -> tokio::process::Command {
    build_tokio_command_with(command, PipeFail::AsShellDefault)
}

fn build_tokio_command_with(command: &str, pipefail: PipeFail) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(shell_program());
    // `as_std_mut()` so the Windows arm can reach `raw_arg` (only defined on
    // `std::process::Command`); the tokio wrapper forwards the raw arg.
    configure_shell_args(cmd.as_std_mut(), command, pipefail);
    // Every shell-family child leads its own process group and dies with the
    // handle that owns it, so a deadline (see
    // `crate::tools::timeout::output_or_kill`) or a cancelled future cannot
    // leave a pipeline running behind the tool that reported it finished.
    cmd.kill_on_drop(true);
    crate::tools::timeout::own_process_group(cmd.as_std_mut());
    cmd
}

/// [`std::process::Command`] variant for adapters that need the platform
/// shell's program and arguments as an owned request.
pub fn build_std_command(command: &str) -> std::process::Command {
    build_std_command_with(command, PipeFail::Surface)
}

/// [`build_std_command`] without changing a shell pipeline's default exit
/// status, for callers whose retry/settlement behavior depends on it.
pub fn build_std_command_preserving_pipe_status(command: &str) -> std::process::Command {
    build_std_command_with(command, PipeFail::AsShellDefault)
}

fn build_std_command_with(command: &str, pipefail: PipeFail) -> std::process::Command {
    let mut cmd = std::process::Command::new(shell_program());
    configure_shell_args(&mut cmd, command, pipefail);
    // Same contract as the tokio builder: the child leads its own process
    // group so a deadline can reach the whole pipeline (see
    // `crate::tools::timeout::kill_process_group`).
    crate::tools::timeout::own_process_group(&mut cmd);
    cmd
}

/// Shell binary for the current platform. Single source of truth shared by
/// [`build_tokio_command`] and [`build_std_command`] — future changes to the
/// platform matrix (adding pwsh, changing pipefail semantics) belong here plus
/// [`configure_shell_args`], so both `Command` flavours stay in lockstep.
fn shell_program() -> &'static str {
    match ShellFlavor::current() {
        ShellFlavor::Cmd => "cmd",
        ShellFlavor::Posix => bash_path().unwrap_or("sh"),
    }
}

/// Append the shell flag + command payload to `cmd`.
///
/// On Windows the payload MUST go through `raw_arg`, not `arg`: Rust's `arg`
/// applies MSVCRT (`CommandLineToArgvW`) quoting, escaping any interior `"` as
/// `\"`. But `cmd.exe` does not understand `\"` — it only toggles quote state
/// on a bare `"`. `raw_arg` passes the string to cmd verbatim. `/C` itself has
/// no special characters.
#[cfg(windows)]
fn configure_shell_args(cmd: &mut std::process::Command, command: &str, _pipefail: PipeFail) {
    use std::os::windows::process::CommandExt;
    // `cmd.exe` has no `pipefail` equivalent, so the flag cannot be honoured
    // here: a Windows pipeline's exit status is its last stage either way.
    cmd.arg("/C").raw_arg(command);
}

/// Unix arm: `bash -lc "set -o pipefail\n<command>"` when bash is present **and**
/// the caller asked to surface pipe-stage failures, else a plain `-lc`.
#[cfg(not(windows))]
fn configure_shell_args(cmd: &mut std::process::Command, command: &str, pipefail: PipeFail) {
    if bash_path().is_some() && pipefail == PipeFail::Surface {
        cmd.arg("-lc").arg(format!("set -o pipefail\n{command}"));
    } else {
        cmd.arg("-lc").arg(command);
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
#[path = "platform_shell_tests.rs"]
mod tests;
