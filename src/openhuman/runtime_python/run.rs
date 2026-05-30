//! Capture-to-completion helper for Python subprocess invocations.
//!
//! `runtime_python::process` is stream-oriented and was sized for the
//! long-lived MCP stdio path. Tools that want the simpler
//! "run-script, write-stdin, read-stdout-and-stderr, return-when-done"
//! contract end up reimplementing the same plumbing (pipe stdin, await
//! `wait_with_output`, bound on a timeout, surface exit code) — this
//! module hoists that into a single helper so every Python-backed tool
//! shares one launch path.

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::time::{timeout, Instant};

use super::bootstrap::ResolvedPython;
use super::process::{spawn_stdio_process, PythonLaunchSpec};

/// Captured output of a one-shot Python subprocess.
#[derive(Debug, Clone)]
pub struct PythonRunOutput {
    /// Process exit code, or `-1` if the child was killed by a signal
    /// (Unix) or the wait failed before exit. Callers should treat
    /// non-zero as failure.
    pub exit_code: i32,
    /// Captured stdout. Decoded as UTF-8 with `from_utf8_lossy` so a
    /// noisy Python script that emits a stray non-UTF-8 byte does not
    /// kill the call.
    pub stdout: String,
    /// Captured stderr, same decoding contract as `stdout`.
    pub stderr: String,
}

/// Error returned when the bounded `run_python_script_to_completion`
/// exceeds its timeout.
#[derive(Debug, thiserror::Error)]
#[error("python script exceeded {timeout_secs}s timeout: {script}")]
pub struct PythonRunTimeout {
    pub script: String,
    pub timeout_secs: u64,
}

/// Spawn a Python subprocess, pipe `stdin` to it (when provided), wait
/// for completion bounded by `deadline`, and return captured output.
///
/// On timeout the child is killed (`kill_on_drop` is set by
/// [`spawn_stdio_process`], so the `Child` going out of scope ends the
/// process) and `Err(PythonRunTimeout)` is returned.
///
/// On a successful spawn but non-zero exit, the call returns `Ok(...)`
/// with the captured stderr and the non-zero `exit_code`. The caller
/// decides how to surface that to the user; we don't promote a
/// non-zero exit to an error here because most consumers want to
/// quote the actual stderr in their own error variant.
pub async fn run_python_script_to_completion(
    resolved: &ResolvedPython,
    spec: &PythonLaunchSpec,
    stdin: Option<Vec<u8>>,
    deadline: Duration,
) -> Result<PythonRunOutput> {
    let started_at = Instant::now();
    let mut child = spawn_stdio_process(resolved, spec)
        .with_context(|| format!("spawning python subprocess for {:?}", spec.script_path))?;

    let timeout_err = || PythonRunTimeout {
        script: spec.script_path.display().to_string(),
        timeout_secs: deadline.as_secs(),
    };

    if let Some(payload) = stdin {
        if let Some(mut stdin_handle) = child.stdin.take() {
            let remaining = deadline.saturating_sub(started_at.elapsed());
            timeout(remaining, stdin_handle.write_all(&payload))
                .await
                .map_err(|_| timeout_err())?
                .with_context(|| {
                    format!(
                        "writing stdin payload to python subprocess for {:?}",
                        spec.script_path
                    )
                })?;
            let remaining = deadline.saturating_sub(started_at.elapsed());
            timeout(remaining, stdin_handle.shutdown())
                .await
                .map_err(|_| timeout_err())?
                .with_context(|| {
                    format!(
                        "closing stdin pipe to python subprocess for {:?}",
                        spec.script_path
                    )
                })?;
        }
    } else if let Some(mut stdin_handle) = child.stdin.take() {
        // Always close stdin so scripts that `sys.stdin.read()` don't
        // deadlock waiting for a payload that's never coming. Bounded
        // by the same deadline so a stuck pipe can't strand us.
        let remaining = deadline.saturating_sub(started_at.elapsed());
        let _ = timeout(remaining, stdin_handle.shutdown()).await;
    }

    let remaining = deadline.saturating_sub(started_at.elapsed());
    let output_future = child.wait_with_output();
    let output = match timeout(remaining, output_future).await {
        Ok(result) => result
            .with_context(|| format!("waiting on python subprocess for {:?}", spec.script_path))?,
        Err(_) => {
            tracing::warn!(
                script = %spec.script_path.display(),
                timeout_secs = deadline.as_secs(),
                "[runtime_python::run] python subprocess exceeded deadline — killed"
            );
            return Err(timeout_err().into());
        }
    };

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    tracing::debug!(
        script = %spec.script_path.display(),
        exit_code,
        stdout_chars = stdout.chars().count(),
        stderr_chars = stderr.chars().count(),
        "[runtime_python::run] python subprocess exited"
    );

    Ok(PythonRunOutput {
        exit_code,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::runtime_python::bootstrap::PythonSource;
    use std::path::PathBuf;

    /// Best-effort PATH lookup for `python3` (with `python` fallback).
    /// Returns `None` when no compatible interpreter is on PATH —
    /// tests use this to skip cleanly on minimal CI images.
    fn locate_python3() -> Option<PathBuf> {
        for name in ["python3", "python"] {
            if let Ok(output) = std::process::Command::new(name).arg("--version").output() {
                if output.status.success() || output.status.code() == Some(0) {
                    return Some(PathBuf::from(name));
                }
            }
        }
        None
    }

    fn resolved_for(bin: &str) -> ResolvedPython {
        ResolvedPython {
            python_bin: PathBuf::from(bin),
            version: "3.12.0".to_string(),
            source: PythonSource::System,
        }
    }

    fn spec_for(script: &str) -> PythonLaunchSpec {
        PythonLaunchSpec::new(PathBuf::from(script))
    }

    #[tokio::test]
    async fn run_returns_err_when_python_binary_missing() {
        let resolved = resolved_for("/definitely/not/a/real/python");
        let spec = spec_for("/tmp/anything.py");
        let err = run_python_script_to_completion(&resolved, &spec, None, Duration::from_secs(1))
            .await
            .expect_err("missing binary must fail spawn");
        assert!(err.to_string().contains("spawning python subprocess"));
    }

    #[tokio::test]
    async fn run_python_inline_round_trips_stdin_and_stdout() {
        // Tolerant: skip when host has no `python3` available so this
        // test stays green on minimal CI images. The full real-Python
        // path is exercised in tests/presentation_tool.rs.
        let python_bin = match locate_python3() {
            Some(p) => p,
            None => {
                eprintln!("skipping: no python3 on PATH");
                return;
            }
        };
        let script = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(
            script.path(),
            "import sys; data = sys.stdin.read(); print(f'echo:{data.strip()}')",
        )
        .expect("write script");

        let resolved = ResolvedPython {
            python_bin: python_bin.clone(),
            version: "3.x".to_string(),
            source: PythonSource::System,
        };
        let spec = PythonLaunchSpec::new(script.path().to_path_buf());

        let output = run_python_script_to_completion(
            &resolved,
            &spec,
            Some(b"hello".to_vec()),
            Duration::from_secs(10),
        )
        .await
        .expect("python run should succeed");

        assert_eq!(output.exit_code, 0, "stderr={}", output.stderr);
        assert!(
            output.stdout.contains("echo:hello"),
            "stdout={}",
            output.stdout
        );
    }

    #[tokio::test]
    async fn run_python_inline_surfaces_nonzero_exit_with_stderr() {
        let python_bin = match locate_python3() {
            Some(p) => p,
            None => {
                eprintln!("skipping: no python3 on PATH");
                return;
            }
        };
        let script = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(
            script.path(),
            "import sys; sys.stderr.write('boom'); sys.exit(2)",
        )
        .expect("write script");

        let resolved = ResolvedPython {
            python_bin,
            version: "3.x".to_string(),
            source: PythonSource::System,
        };
        let spec = PythonLaunchSpec::new(script.path().to_path_buf());

        let output =
            run_python_script_to_completion(&resolved, &spec, None, Duration::from_secs(10))
                .await
                .expect("non-zero exit is Ok with output, not Err");

        assert_eq!(output.exit_code, 2);
        assert!(output.stderr.contains("boom"), "stderr={}", output.stderr);
    }

    #[tokio::test]
    async fn run_python_inline_times_out_on_runaway_script() {
        let python_bin = match locate_python3() {
            Some(p) => p,
            None => {
                eprintln!("skipping: no python3 on PATH");
                return;
            }
        };
        let script = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(script.path(), "import time; time.sleep(30)").expect("write script");

        let resolved = ResolvedPython {
            python_bin,
            version: "3.x".to_string(),
            source: PythonSource::System,
        };
        let spec = PythonLaunchSpec::new(script.path().to_path_buf());

        let err =
            run_python_script_to_completion(&resolved, &spec, None, Duration::from_millis(250))
                .await
                .expect_err("timeout must surface");
        assert!(err.to_string().contains("exceeded"));
    }
}
