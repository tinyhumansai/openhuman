//! Result mapping and child-environment helpers for the `shell` tool.

use std::time::Duration;
use tinytools::ToolResult;

/// Exit status coreutils `timeout` returns when its own limit expires.
pub(super) const COMMAND_TIMEOUT_EXIT_CODE: i32 = 124;
/// Appended to an exit-124 failure: the command's own `timeout` fired, not the
/// tool's (#6953). Without it, `exit code 124 [stdout] ok` reads like a bug.
pub(super) const COMMAND_TIMEOUT_NOTE: &str = "[exit 124: the command's own `timeout` limit expired; the shell tool's timeout_secs did not fire]";

/// The result when the tool's own `timeout_secs` deadline killed the command.
pub(super) fn tool_timeout_message(secs: u64) -> String {
    format!("Command timed out after {secs}s and was killed: the shell tool's timeout_secs limit fired.")
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
            ToolResult::error(tool_timeout_message(effective.as_secs()))
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

/// Python's text-encoding defaults for a child, unless the parent sets its own.
/// Under `cmd.exe` the console code page is a legacy one (cp1252, cp437), so a
/// script printing anything outside it dies with `UnicodeEncodeError`. Both are
/// no-ops for a non-Python command. Native runs get the same from TinyBox's
/// `python_utf8` request flag; this serves the Docker and sandbox-policy paths,
/// which forward their environment as plain variables.
pub(super) const PYTHON_UTF8_DEFAULTS: &[(&str, &str)] =
    &[("PYTHONUTF8", "1"), ("PYTHONIOENCODING", "utf-8")];

pub(super) fn python_utf8_env(
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<(&'static str, std::ffi::OsString)> {
    PYTHON_UTF8_DEFAULTS
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
                tinytools_std::timeout::CommandEnvironment::var_os(name).map(|value| (*name, value))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    }
    .into_iter()
    .chain(python_utf8_env(|name| {
        explicit
            .and_then(|overrides| overrides.get(name).map(std::ffi::OsString::from))
            .or_else(|| {
                explicit
                    .is_none()
                    .then(|| tinytools_std::timeout::CommandEnvironment::var_os(name))
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
