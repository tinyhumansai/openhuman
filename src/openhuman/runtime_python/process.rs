//! Python child-process launch helpers.
//!
//! Uses unbuffered stdio (`-u`) by default so line-oriented protocols such as
//! MCP do not stall behind Python's output buffering.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;

use super::bootstrap::ResolvedPython;

/// Launch spec for a Python stdio subprocess.
#[derive(Debug, Clone)]
pub struct PythonLaunchSpec {
    /// Absolute or caller-resolved path to the Python script.
    pub script_path: PathBuf,
    /// Positional arguments forwarded after the script path.
    pub args: Vec<String>,
    /// Optional working directory for the child process.
    pub cwd: Option<PathBuf>,
    /// Extra environment variables to set on the child process.
    pub env: BTreeMap<String, String>,
    /// Whether to pass `-u` for unbuffered stdio. Defaults to `true`.
    pub unbuffered: bool,
}

impl PythonLaunchSpec {
    pub fn new(script_path: PathBuf) -> Self {
        Self {
            script_path,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            unbuffered: true,
        }
    }
}

pub fn spawn_stdio_process(
    resolved: &ResolvedPython,
    spec: &PythonLaunchSpec,
) -> Result<tokio::process::Child> {
    let mut cmd = tokio::process::Command::new(&resolved.python_bin);
    if spec.unbuffered {
        cmd.arg("-u");
    }
    cmd.arg(&spec.script_path);
    cmd.args(&spec.args);
    if let Some(cwd) = spec.cwd.as_ref() {
        cmd.current_dir(cwd);
    }
    for (key, value) in &spec.env {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = cmd.spawn().with_context(|| {
        format!(
            "failed to spawn python process `{}` for script {}",
            resolved.python_bin.display(),
            spec.script_path.display()
        )
    })?;

    tracing::info!(
        python_bin = %resolved.python_bin.display(),
        script = %spec.script_path.display(),
        arg_count = spec.args.len(),
        cwd = spec.cwd.as_ref().map(|p| p.display().to_string()),
        "[runtime_python::process] spawned stdio python child"
    );

    Ok(child)
}
