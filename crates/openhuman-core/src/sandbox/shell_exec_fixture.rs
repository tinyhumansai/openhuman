//! Test-only stand-in for the module's native shell: unit tests do not load a
//! native TinyBox artifact, so unjailed runs execute here. The module's own
//! behaviour (shell selection, environment, scratch) is covered by its crate and
//! by the released-artifact test; this fixture only has to run a command, and
//! like the module it keeps what a command printed before its deadline stopped
//! it.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

use super::{NativeShellRun, SandboxExecResult, ShellRunOutcome};
use crate::sandbox::shell_live::{LiveHandle, LiveRead, LiveShell, LiveState};

/// Variables the module's safe environment snapshot carries.
const SNAPSHOT: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];

pub(crate) async fn run(run: NativeShellRun) -> anyhow::Result<ShellRunOutcome> {
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
    tinyagents_harness_tools::exec::timeout::own_process_group(command.as_std_mut());
    let mut child = command
        .spawn()
        .map_err(|error| anyhow::anyhow!("Failed to execute command: {error}"))?;
    let pid = child.id();
    // Read both streams as the command runs, like the module does, so a
    // command stopped at its deadline keeps what it printed before.
    let (mut stdout_pipe, mut stderr_pipe) = (child.stdout.take(), child.stderr.take());
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let started = tokio::time::Instant::now();
    // Like the module: a command still running at its yield point is handed
    // over running, with what it printed so far.
    let yield_after = run.yield_after.filter(|after| *after < run.timeout);
    let finished = tokio::time::timeout(yield_after.unwrap_or(run.timeout), async {
        let (status, (), ()) = tokio::join!(
            child.wait(),
            drain(&mut stdout_pipe, &mut stdout),
            drain(&mut stderr_pipe, &mut stderr),
        );
        status
    })
    .await;
    let (exit_code, timed_out) = match finished {
        Ok(status) => (status?.code().unwrap_or(-1), false),
        Err(_) if yield_after.is_some() => {
            return Ok(ShellRunOutcome::Running(LiveShell {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                handle: LiveHandle::Fixture(FixtureLive {
                    child,
                    pid,
                    stdout_pipe,
                    stderr_pipe,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    deadline: started + run.timeout,
                    ended: false,
                }),
            }));
        }
        Err(_) => {
            // Stop the command and everything it started, then give the
            // readers a moment to drain what is left in the pipes.
            if let Some(pid) = pid {
                tinyagents_harness_tools::exec::timeout::kill_process_group(pid);
            }
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                tokio::join!(
                    drain(&mut stdout_pipe, &mut stdout),
                    drain(&mut stderr_pipe, &mut stderr),
                )
            })
            .await;
            (-1, true)
        }
    };
    // The module streams output into the caller's meter as it reads it; the
    // fixture folds the collected output in the same way.
    if let Some(meter) = &run.meter {
        meter.record(&stdout);
        meter.record(&stderr);
    }
    Ok(ShellRunOutcome::Finished(SandboxExecResult {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
    }))
}

/// A fixture command past its yield point, read and stopped like the module's.
pub(crate) struct FixtureLive {
    child: tokio::process::Child,
    pid: Option<u32>,
    stdout_pipe: Option<tokio::process::ChildStdout>,
    stderr_pipe: Option<tokio::process::ChildStderr>,
    /// Output read since the previous [`LiveRead`].
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    deadline: tokio::time::Instant,
    ended: bool,
}

impl FixtureLive {
    pub(crate) async fn read(&mut self, wait: Duration) -> anyhow::Result<LiveRead> {
        if self.ended {
            return Ok(self.take(LiveState::Stopped));
        }
        let until = (tokio::time::Instant::now() + wait).min(self.deadline);
        let finished = tokio::time::timeout_at(until, async {
            let (status, (), ()) = tokio::join!(
                self.child.wait(),
                drain(&mut self.stdout_pipe, &mut self.stdout),
                drain(&mut self.stderr_pipe, &mut self.stderr),
            );
            status
        })
        .await;
        let state = match finished {
            Ok(status) => {
                // Ended only once the wait itself succeeded.
                let code = status?.code().unwrap_or(-1);
                self.ended = true;
                LiveState::Exited(code)
            }
            Err(_) if tokio::time::Instant::now() >= self.deadline => {
                self.stop().await;
                LiveState::TimedOut
            }
            Err(_) => LiveState::Running,
        };
        Ok(self.take(state))
    }

    pub(crate) fn finished(&self) -> bool {
        self.ended
    }

    pub(crate) async fn kill(&mut self) -> anyhow::Result<LiveRead> {
        if !self.ended {
            self.stop().await;
        }
        Ok(self.take(LiveState::Stopped))
    }

    /// Stop the command and everything it started, keeping what is left in
    /// the pipes.
    async fn stop(&mut self) {
        if let Some(pid) = self.pid {
            tinyagents_harness_tools::exec::timeout::kill_process_group(pid);
        }
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                drain(&mut self.stdout_pipe, &mut self.stdout),
                drain(&mut self.stderr_pipe, &mut self.stderr),
            )
        })
        .await;
        self.ended = true;
    }

    fn take(&mut self, state: LiveState) -> LiveRead {
        LiveRead {
            stdout: String::from_utf8_lossy(&std::mem::take(&mut self.stdout)).into_owned(),
            stderr: String::from_utf8_lossy(&std::mem::take(&mut self.stderr)).into_owned(),
            state,
        }
    }
}

impl Drop for FixtureLive {
    fn drop(&mut self) {
        if !self.ended {
            if let Some(pid) = self.pid {
                tinyagents_harness_tools::exec::timeout::kill_process_group(pid);
            }
            let _ = self.child.start_kill();
        }
    }
}

/// Appends what `pipe` delivers to `into` until it closes. Reads are
/// cancel-safe, so a drain cut short by the deadline keeps every chunk it read,
/// and a later call picks up where it stopped.
async fn drain(pipe: &mut Option<impl AsyncRead + Unpin>, into: &mut Vec<u8>) {
    let Some(pipe) = pipe else {
        return;
    };
    let mut chunk = [0u8; 8192];
    while let Ok(read) = pipe.read(&mut chunk).await {
        if read == 0 {
            break;
        }
        into.extend_from_slice(&chunk[..read]);
    }
}
