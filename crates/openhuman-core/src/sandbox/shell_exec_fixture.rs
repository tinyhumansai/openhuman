//! Test-only stand-in for the module's native shell: unit tests do not load a
//! native TinyBox artifact, so unjailed runs execute here. The module's own
//! behaviour (shell selection, environment, scratch) is covered by its crate and
//! by the released-artifact test; this fixture only has to run a command.

use std::process::Stdio;

use super::{NativeShellRun, SandboxExecResult};

/// Variables the module's safe environment snapshot carries.
const SNAPSHOT: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];

pub(crate) async fn run(run: NativeShellRun) -> anyhow::Result<SandboxExecResult> {
    let bash = ["/usr/bin/bash", "/bin/bash"]
        .into_iter()
        .find(|path| std::path::Path::new(path).exists());
    let mut command = tokio::process::Command::new(if cfg!(windows) {
        "cmd"
    } else {
        bash.unwrap_or("sh")
    });
    if cfg!(windows) {
        command.arg("/C").arg(&run.command);
    } else if bash.is_some() && !run.preserve_pipeline_status {
        command
            .arg("-c")
            .arg(format!("set -o pipefail\n{}", run.command));
    } else {
        command.arg("-c").arg(&run.command);
    }
    command.env_clear();
    for name in SNAPSHOT {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if cfg!(windows) {
        for name in ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
    }
    if run.python_utf8 {
        // The module defaults these unless the parent environment sets them.
        for (name, default) in [("PYTHONUTF8", "1"), ("PYTHONIOENCODING", "utf-8")] {
            command.env(
                name,
                std::env::var_os(name).unwrap_or_else(|| default.into()),
            );
        }
    }
    command.envs(&run.env);
    command
        .current_dir(&run.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    tinytools_std::timeout::own_process_group(command.as_std_mut());
    match tinytools_std::timeout::output_or_kill(&mut command, run.timeout).await {
        Ok(Ok(output)) => {
            // The module streams output into the caller's meter as it reads it;
            // the fixture folds the finished output in the same way.
            if let Some(meter) = &run.meter {
                meter.record(&output.stdout);
                meter.record(&output.stderr);
            }
            Ok(SandboxExecResult {
                exit_code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                timed_out: false,
            })
        }
        Ok(Err(error)) => anyhow::bail!("Failed to execute command: {error}"),
        Err(_) => Ok(SandboxExecResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("Command timed out after {}s", run.timeout.as_secs()),
            timed_out: true,
        }),
    }
}
