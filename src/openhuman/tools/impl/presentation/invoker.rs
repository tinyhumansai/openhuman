//! Python-invocation seam for the presentation tool.
//!
//! The production [`RealPythonInvoker`] wires the tool through
//! `runtime_python::venv::ensure_venv` + `run_python_script_to_completion`.
//! Unit tests in `tests.rs` inject a `MockPythonInvoker` so coverage on
//! the success / non-zero-exit / timeout / missing-runtime branches
//! does not require a live Python interpreter in CI.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::openhuman::config::schema::RuntimePythonConfig;
use crate::openhuman::runtime_python::{
    ensure_venv, run_python_script_to_completion, PythonLaunchSpec, PythonRunOutput,
    PythonRunTimeout,
};

/// Outcome of a single bundled-script run, normalised across all
/// invoker impls.
#[derive(Debug, Clone)]
pub(super) enum InvocationOutcome {
    /// Script returned exit 0 — caller still needs to parse stdout
    /// for the in-band JSON status.
    Success { stdout: String, stderr: String },
    /// Script exited non-zero. `stderr` is already truncated by the
    /// tool layer before it reaches the agent.
    NonZeroExit {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    /// Process exceeded the deadline and was killed.
    Timeout { timeout_secs: u64 },
    /// `runtime_python` could not resolve a Python interpreter at all
    /// (e.g. `runtime_python.enabled = false`, no system Python, no
    /// network for managed install). Distinct from `NonZeroExit` so
    /// the tool surfaces `MissingRuntime` with an actionable install
    /// hint instead of a noisy stderr dump.
    MissingRuntime { reason: String },
    /// venv setup or `pip install` failed. Distinct from
    /// `NonZeroExit` so the tool surfaces `MissingPackage` with an
    /// install hint.
    PackageInstallFailed { reason: String },
}

/// Boundary trait for invoking the bundled `generate_pptx.py` script.
/// Implementors own venv setup, subprocess spawn, stdin piping, and
/// stdout/stderr capture.
#[async_trait]
pub(super) trait PythonInvoker: Send + Sync {
    async fn run(
        &self,
        script_path: &Path,
        stdin_payload: Vec<u8>,
        output_path: &Path,
        deadline: Duration,
    ) -> Result<InvocationOutcome>;
}

/// Production invoker — bridges to `runtime_python` for venv setup +
/// subprocess execution. Each call re-runs `ensure_venv`; the venv
/// itself is cached on disk so the second call short-circuits inside
/// `ensure_venv` without spawning pip again.
pub(super) struct RealPythonInvoker {
    runtime_python: RuntimePythonConfig,
}

impl RealPythonInvoker {
    pub(super) fn new(runtime_python: RuntimePythonConfig) -> Self {
        Self { runtime_python }
    }
}

/// Name of the on-disk venv that holds python-pptx. Lives at
/// `<runtime_python.cache_dir>/venvs/presentation-pptx/`.
const VENV_NAME: &str = "presentation-pptx";

/// Pinned python-pptx version. Update intentionally — bumping minor
/// versions historically broke layout placeholder lookups (see the
/// project changelog).
const PPTX_REQUIREMENT: &str = "python-pptx==1.0.2";

#[async_trait]
impl PythonInvoker for RealPythonInvoker {
    async fn run(
        &self,
        script_path: &Path,
        stdin_payload: Vec<u8>,
        output_path: &Path,
        deadline: Duration,
    ) -> Result<InvocationOutcome> {
        if !self.runtime_python.enabled {
            return Ok(InvocationOutcome::MissingRuntime {
                reason: "runtime_python is disabled in config".to_string(),
            });
        }

        let resolved = match ensure_venv(VENV_NAME, &[PPTX_REQUIREMENT], &self.runtime_python).await
        {
            Ok(r) => r,
            Err(err) => {
                let msg = err.to_string();
                tracing::warn!(
                    venv = VENV_NAME,
                    reason = %msg,
                    "[presentation::invoker] venv setup failed"
                );
                if msg.contains("runtime_python is disabled")
                    || msg.contains("no python")
                    || msg.contains("resolving base python")
                {
                    return Ok(InvocationOutcome::MissingRuntime { reason: msg });
                }
                return Ok(InvocationOutcome::PackageInstallFailed { reason: msg });
            }
        };

        let mut spec = PythonLaunchSpec::new(script_path.to_path_buf());
        spec.args = vec!["--output".to_string(), path_to_string(output_path)];

        let run_result =
            run_python_script_to_completion(&resolved, &spec, Some(stdin_payload), deadline).await;
        let output: PythonRunOutput = match run_result {
            Ok(o) => o,
            Err(err) => {
                if err.downcast_ref::<PythonRunTimeout>().is_some() {
                    return Ok(InvocationOutcome::Timeout {
                        timeout_secs: deadline.as_secs(),
                    });
                }
                // Spawn errors etc. — surface as MissingRuntime so the
                // agent treats it as an environment problem, not a
                // user-input problem.
                return Ok(InvocationOutcome::MissingRuntime {
                    reason: err.to_string(),
                });
            }
        };

        if output.exit_code == 0 {
            Ok(InvocationOutcome::Success {
                stdout: output.stdout,
                stderr: output.stderr,
            })
        } else {
            Ok(InvocationOutcome::NonZeroExit {
                exit_code: output.exit_code,
                stdout: output.stdout,
                stderr: output.stderr,
            })
        }
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
