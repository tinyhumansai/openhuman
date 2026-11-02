//! Result mapping and child-environment helpers for the `shell` tool.

use std::time::Duration;
use tinytools::ToolResult;

/// Exit status coreutils `timeout` returns when its own limit expires.
pub(super) const COMMAND_TIMEOUT_EXIT_CODE: i32 = 124;
/// Appended to an exit-124 failure: the command's own `timeout` fired, not the
/// tool's (#6953). Without it, `exit code 124 [stdout] ok` reads like a bug.
pub(super) const COMMAND_TIMEOUT_NOTE: &str =
    "[exit 124: the command's own `timeout` limit expired; the shell tool's deadline did not fire]";

/// The result when the tool's deadline killed the command: in a turn, the
/// turn's remaining budget (TinyAgents' `ShellTurnBudget` clamp); outside one,
/// the caller's `timeout_secs`. It stays the first line of the result:
/// TinyAgents' shell turn-budget note recognises a fired deadline by this
/// prefix.
pub(super) fn tool_timeout_message(secs: u64) -> String {
    format!("Command timed out after {secs}s and was killed: the shell tool's deadline fired.")
}

/// [`tinytools::command_failure`], plus [`COMMAND_TIMEOUT_NOTE`] on exit 124.
pub(super) fn command_failure(code: Option<i32>, stdout: &str, stderr: &str) -> ToolResult {
    let mut failure = tinytools::command_failure(code, stdout, stderr);
    if code == Some(COMMAND_TIMEOUT_EXIT_CODE) {
        tracing::debug!("[shell] exit 124: attributing it to the command's own timeout");
        failure.content.push(tinytools::ToolContent::Text {
            text: COMMAND_TIMEOUT_NOTE.to_string(),
        });
    }
    failure
}

/// Map a finished sandbox or module execution to the tool result.
///
/// The same exit-code and both-streams surfacing everywhere (#4095); the `-1`
/// sentinel renders as a signal.
pub(super) fn exec_result(
    result: anyhow::Result<crate::sandbox::SandboxExecResult>,
    effective: Duration,
) -> ToolResult {
    match result {
        Ok(result) if result.timed_out => {
            ToolResult::error(timed_out_output(effective.as_secs(), &result))
        }
        Ok(result) if result.success() => {
            if result.stderr.is_empty() {
                ToolResult::success(result.stdout)
            } else {
                ToolResult::success(format!("{}\n[stderr]\n{}", result.stdout, result.stderr))
            }
        }
        Ok(result) => command_failure(
            tinytools::sandbox_exit_code(result.exit_code),
            &result.stdout,
            &result.stderr,
        ),
        Err(e) => ToolResult::error(format!("Sandbox execution failed: {e}")),
    }
}

/// The timeout line, then what the command printed before it was killed.
///
/// A test or build that runs past its deadline has usually already printed
/// what the model needs: its first failures, how far it got, the error that
/// stalled it. Dropping that leaves only "timed out", and the model reruns
/// blind. When nothing came back, the result says why output can be missing
/// (a pipeline such as `| tail`, or a program that buffers its output, holds
/// it until the command exits), so the model does not read it as a hang.
fn timed_out_output(secs: u64, result: &crate::sandbox::SandboxExecResult) -> String {
    let mut out = tool_timeout_message(secs);
    let stdout = result.stdout.trim_end();
    let stderr = result.stderr.trim_end();
    if stdout.is_empty() && stderr.is_empty() {
        out.push_str(
            " No output reached the tool before the kill; a pipeline such as `| tail` \
             or a program that buffers its output holds it until the command exits.",
        );
        return out;
    }
    tracing::debug!(
        stdout_bytes = stdout.len(),
        stderr_bytes = stderr.len(),
        "[shell] timed out: returning the output printed before the kill"
    );
    out.push_str(" Output before the kill:");
    if !stdout.is_empty() {
        out.push_str("\n[stdout]\n");
        out.push_str(stdout);
    }
    if !stderr.is_empty() {
        out.push_str("\n[stderr]\n");
        out.push_str(stderr);
    }
    out
}

/// Python defaults for a child, unless the parent sets its own.
///
/// - `PYTHONUTF8`, `PYTHONIOENCODING`: under `cmd.exe` the console code page
///   is a legacy one (cp1252, cp437), so a script printing anything outside it
///   dies with `UnicodeEncodeError`. Native runs get these from TinyBox's
///   `python_utf8` request flag.
/// - `PYTHONUNBUFFERED`: Python block-buffers stdout into a pipe (about 8 KB),
///   and a process killed at its deadline never flushes that buffer, so a
///   script that printed its first failures and then ran long would show
///   nothing. Any non-empty value turns buffering off; a parent that wants it
///   on sets it to the empty string.
///
/// All are no-ops for a non-Python command. This serves the Docker and
/// sandbox-policy paths, which forward their environment as plain variables;
/// the native path adds `PYTHONUNBUFFERED` itself.
pub(super) const PYTHON_ENV_DEFAULTS: &[(&str, &str)] = &[
    ("PYTHONUTF8", "1"),
    ("PYTHONIOENCODING", "utf-8"),
    (PYTHON_UNBUFFERED, "1"),
];

/// The variable that turns off Python's output buffering.
pub(super) const PYTHON_UNBUFFERED: &str = "PYTHONUNBUFFERED";

/// A native run's environment with `PYTHONUNBUFFERED` added, so a script killed
/// at its deadline has written what it printed (TinyBox's `python_utf8` flag
/// covers the encoding). A value already in `env` stays. Otherwise the
/// process's own value is inherited, unless the turn owns its environment
/// (`turn_environment`), as on the sandbox path; else it is `1`.
pub(super) fn with_unbuffered_python(
    mut env: std::collections::BTreeMap<String, String>,
    turn_environment: bool,
) -> std::collections::BTreeMap<String, String> {
    if !env.contains_key(PYTHON_UNBUFFERED) {
        let inherited = (!turn_environment)
            .then(|| std::env::var(PYTHON_UNBUFFERED).ok())
            .flatten();
        env.insert(
            PYTHON_UNBUFFERED.to_owned(),
            inherited.unwrap_or_else(|| "1".to_owned()),
        );
    }
    env
}

/// Point a native child's temp dir at the agent's granted scratch dir
/// (`/tmp/openhuman`, a ReadWrite trusted root — see SecurityPolicy
/// `from_config`) so `python3 tempfile` / `mktemp` / `$TMPDIR` writes land in a
/// sandboxed, readable location instead of the world-shared /tmp.
pub(super) fn use_scratch_tmp(env: &mut std::collections::BTreeMap<String, String>) {
    let scratch_dir = crate::security::openhuman_scratch_dir();
    if let (true, Some(scratch)) = (scratch_dir.is_dir(), scratch_dir.to_str()) {
        tracing::debug!(
            scratch_dir = %scratch_dir.display(),
            "[shell] overriding TMPDIR/TMP/TEMP to the openhuman scratch dir"
        );
        for name in ["TMPDIR", "TMP", "TEMP"] {
            env.insert(name.to_owned(), scratch.to_owned());
        }
    } else {
        tracing::debug!(
            scratch_dir = %scratch_dir.display(),
            "[shell] scratch dir missing — leaving TMPDIR/TMP/TEMP to the module"
        );
    }
}

/// The [`PYTHON_ENV_DEFAULTS`] a child gets, each one the parent's own value
/// when `lookup` has one.
pub(super) fn python_env_defaults(
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<(&'static str, std::ffi::OsString)> {
    PYTHON_ENV_DEFAULTS
        .iter()
        .map(|(name, default)| {
            let value = lookup(name).unwrap_or_else(|| (*default).into());
            (*name, value)
        })
        .collect()
}

/// The host environment a Docker container receives: the sandbox allowlist from
/// the parent plus the Python encoding defaults. `None` when a value is not UTF-8.
#[cfg(feature = "modules")]
pub(super) fn container_environment(
    explicit: Option<&std::collections::BTreeMap<String, String>>,
) -> Option<std::collections::BTreeMap<String, String>> {
    let mut env = std::collections::BTreeMap::new();
    if let Some(explicit) = explicit {
        env.extend(
            explicit
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
    }
    let inherited = if explicit.is_none() {
        crate::sandbox::ops::SANDBOX_ENV_PASSTHROUGH
            .iter()
            .filter_map(|name| {
                tinyagents_harness_tools::exec::timeout::CommandEnvironment::var_os(name)
                    .map(|value| (*name, value))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    }
    .into_iter()
    .chain(python_env_defaults(|name| {
        explicit
            .and_then(|overrides| overrides.get(name).map(std::ffi::OsString::from))
            .or_else(|| {
                explicit
                    .is_none()
                    .then(|| {
                        tinyagents_harness_tools::exec::timeout::CommandEnvironment::var_os(name)
                    })
                    .flatten()
            })
    }));
    for (name, value) in inherited {
        env.insert(name.to_owned(), value.into_string().ok()?);
    }
    Some(env)
}

/// Add the explicit turn environment to a sandbox command request.
pub(super) fn extend_explicit_environment(
    environment: &mut std::collections::HashMap<std::ffi::OsString, std::ffi::OsString>,
    explicit: Option<&std::collections::BTreeMap<String, String>>,
) {
    if let Some(explicit) = explicit {
        environment.extend(explicit.iter().map(|(name, value)| {
            (
                std::ffi::OsString::from(name),
                std::ffi::OsString::from(value),
            )
        }));
    }
}

/// The argument a turn's `shell` call carries its yield point in. The model
/// writes it as `timeout_secs`; the turn moves it here before TinyAgents'
/// turn-budget clamp writes the command's deadline into `timeout_secs`
/// (`agent::tinyagents::middleware::shell_yield`).
pub(crate) const YIELD_AFTER_ARG: &str = "yield_after_secs";
/// When a command still running is handed back as a process, unless the call
/// says otherwise.
pub(super) const DEFAULT_YIELD_AFTER_SECS: u64 = 120;
/// The latest a call may ask to come back.
pub(super) const MAX_YIELD_AFTER_SECS: u64 = 600;
/// The earliest the deadline-relative cap brings the yield point forward to.
pub(super) const MIN_YIELD_AFTER_SECS: u64 = 30;

/// The yield point for `command` with `deadline` to run: the requested time
/// (default 120 s, at most 600 s), but no later than an eighth of the deadline
/// (and not earlier than 30 s for that cap), and always before the deadline
/// itself, so a command started late in a turn still comes back before the
/// turn's deadline kills it. None for `0`, and none for a deliberate wait
/// ([`is_deliberate_wait`]), which would only come back with nothing to show.
pub(super) fn yield_after(
    requested: Option<u64>,
    deadline: Duration,
    command: &str,
) -> Option<Duration> {
    let secs = requested
        .unwrap_or(DEFAULT_YIELD_AFTER_SECS)
        .min(MAX_YIELD_AFTER_SECS);
    if secs == 0 || is_deliberate_wait(command) {
        return None;
    }
    let deadline = deadline.as_secs();
    let cap = (deadline / 8).max(MIN_YIELD_AFTER_SECS).min(deadline / 2);
    (cap > 0).then(|| Duration::from_secs(secs.min(cap)))
}

/// Commands that only look at the state a wait led to: reading a log,
/// listing files, checking a process.
const QUICK_LOOKS: &[&str] = &[
    "cat", "date", "df", "du", "echo", "grep", "head", "ls", "pgrep", "printf", "ps", "stat",
    "tail", "test", "wc",
];

/// A command that starts with `sleep` and then only looks at what happened
/// (`sleep 60; tail -20 build.log`). It is meant to wait, so it is not cut
/// short. `sleep 2; make -j8` starts real work after the sleep and yields as
/// usual.
pub(super) fn is_deliberate_wait(command: &str) -> bool {
    let mut steps = command
        .split(['\n', ';', '&', '|'])
        .map(str::trim)
        .filter(|step| !step.is_empty());
    let first_word = |step: &str| step.split_whitespace().next().unwrap_or("").to_owned();
    if steps.next().map(first_word).as_deref() != Some("sleep") {
        return false;
    }
    steps.all(|step| QUICK_LOOKS.contains(&first_word(step).as_str()))
}

/// The result of a call that came back with its command still running.
pub(super) fn yielded_output(
    id: &str,
    elapsed: Duration,
    timeout: Duration,
    stdout: &str,
    stderr: &str,
) -> String {
    let mut out = format!(
        "Still running as process {id} after {}; it keeps running (it stops at its {} timeout).",
        human_duration(elapsed),
        human_duration(timeout),
    );
    out.push_str(&printed(stdout, stderr, "Output so far:", "No output yet."));
    out.push_str(&format!(
        "\nFollow it with shell_process {{\"process\": \"{id}\"}} to wait for more output or the \
         end, or {{\"process\": \"{id}\", \"action\": \"kill\"}} to stop it. Do not start the \
         command again while it runs."
    ));
    out
}

/// `label` and the non-empty streams, or `empty` when both are empty.
pub(super) fn printed(stdout: &str, stderr: &str, label: &str, empty: &str) -> String {
    let (stdout, stderr) = (stdout.trim_end(), stderr.trim_end());
    if stdout.is_empty() && stderr.is_empty() {
        return format!(" {empty}");
    }
    let mut out = format!(" {label}");
    if !stdout.is_empty() {
        out.push_str("\n[stdout]\n");
        out.push_str(stdout);
    }
    if !stderr.is_empty() {
        out.push_str("\n[stderr]\n");
        out.push_str(stderr);
    }
    out
}

/// `45s`, `2m 0s`, `1h 5m`.
pub(super) fn human_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

#[cfg(test)]
#[path = "shell_support_tests.rs"]
mod tests;
