//! On-demand virtualenv manager for Python-backed tools.
//!
//! [`PythonBootstrap`] resolves a base interpreter; this module layers
//! a per-tool venv on top so individual tools can declare their own
//! requirements (e.g. `python-pptx` for the presentation tool) without
//! polluting the user's system site-packages and without each tool
//! reimplementing `python -m venv` + `pip install`.
//!
//! Venvs live under `<runtime_python.cache_dir>/venvs/<name>/`. First
//! call to [`ensure_venv`] creates the venv and pip-installs the
//! requested packages; subsequent calls short-circuit when the
//! recorded `requirements.txt` matches.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use super::bootstrap::{PythonBootstrap, PythonSource, ResolvedPython};
use super::process::{spawn_stdio_process, PythonLaunchSpec};
use crate::openhuman::config::schema::RuntimePythonConfig;

/// Hard upper bound on how long the first `python -m venv` +
/// `pip install` may run before the install is abandoned. First call
/// on a cold pip cache is the slow path; 5 minutes covers a typical
/// `python-pptx` install on a residential network.
const VENV_INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Filename of the recorded requirements set inside each venv dir.
/// Used to detect when the caller's requirement list has drifted from
/// what the venv was last seeded with.
const REQUIREMENTS_LOCK: &str = "requirements.txt";

/// Ensure a managed virtualenv named `name` exists under
/// `<cache_root>/venvs/<name>/` and has `requirements` installed.
/// Returns a [`ResolvedPython`] pointing at the venv's `bin/python`.
///
/// First-call semantics: resolves the base interpreter via
/// [`PythonBootstrap::resolve`], creates the venv with
/// `python -m venv <path>`, runs `pip install <requirements...>`
/// (bounded by [`VENV_INSTALL_TIMEOUT`]), writes the rendered
/// requirements list to `<venv>/requirements.txt` for change
/// detection, and returns the venv interpreter path.
///
/// Subsequent calls: if the venv exists and `requirements.txt`
/// matches the current call's requirements list, return the cached
/// path without spawning pip. Mismatched lists trigger a fresh
/// install (the existing venv is wiped first so we don't end up with
/// stale extras).
pub async fn ensure_venv(
    name: &str,
    requirements: &[&str],
    config: &RuntimePythonConfig,
) -> Result<ResolvedPython> {
    validate_venv_name(name)?;
    let cache_root = resolve_cache_root(config);
    let venv_dir = cache_root.join("venvs").join(name);
    let lock_path = venv_dir.join(REQUIREMENTS_LOCK);
    let want_lock = render_requirements_lock(requirements);

    if let Some(resolved) = try_cached_venv(&venv_dir, &lock_path, &want_lock).await {
        tracing::debug!(
            name = name,
            venv = %venv_dir.display(),
            "[runtime_python::venv] reusing cached venv"
        );
        return Ok(resolved);
    }

    let bootstrap = PythonBootstrap::new(config.clone());
    let base = bootstrap.resolve().await.context("resolving base python")?;

    // Log the count, not the raw requirement strings — a caller can
    // legitimately pass `pkg @ https://user:token@host/...` style
    // entries (private index URLs, credentialed git refs) and we
    // don't want those leaking into the structured logs.
    tracing::info!(
        name = name,
        base_python = %base.python_bin.display(),
        venv = %venv_dir.display(),
        requirement_count = requirements.len(),
        "[runtime_python::venv] first-call setup — creating venv + pip install (one-time, ~30s)"
    );

    if venv_dir.exists() {
        tokio::fs::remove_dir_all(&venv_dir)
            .await
            .with_context(|| format!("removing stale venv dir {}", venv_dir.display()))?;
    }
    if let Some(parent) = venv_dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating venv parent {}", parent.display()))?;
    }

    create_venv(&base, &venv_dir).await?;
    let venv_python = locate_venv_python(&venv_dir).ok_or_else(|| {
        anyhow!(
            "venv created but no python binary found under {}",
            venv_dir.display()
        )
    })?;
    pip_install(&venv_python, requirements).await?;

    tokio::fs::write(&lock_path, &want_lock)
        .await
        .with_context(|| format!("writing requirements lock {}", lock_path.display()))?;

    let version = base.version.clone();
    Ok(ResolvedPython {
        python_bin: venv_python,
        version,
        source: PythonSource::Managed,
    })
}

/// True iff `name` is safe to use as a venv directory component.
/// Mirrors the artifact-id rules: no path traversal, no separators,
/// no absolute paths.
fn validate_venv_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("venv name must not be empty");
    }
    if name == "." || name == ".." {
        bail!("venv name must not be '.' or '..'");
    }
    if name.contains('/') || name.contains('\\') {
        bail!("venv name must not contain path separators: {name:?}");
    }
    Ok(())
}

fn resolve_cache_root(config: &RuntimePythonConfig) -> PathBuf {
    let configured = config.cache_dir.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    if let Some(user_cache) = dirs::cache_dir() {
        return user_cache.join("openhuman").join("runtime-python");
    }
    PathBuf::from(".openhuman").join("runtime-python")
}

async fn try_cached_venv(
    venv_dir: &Path,
    lock_path: &Path,
    want_lock: &str,
) -> Option<ResolvedPython> {
    if !venv_dir.is_dir() {
        return None;
    }
    let venv_python = locate_venv_python(venv_dir)?;
    let recorded = tokio::fs::read_to_string(lock_path).await.ok()?;
    if recorded != want_lock {
        return None;
    }
    Some(ResolvedPython {
        python_bin: venv_python,
        // Version is not re-probed here — the cached venv is by
        // construction the same minor as the base interpreter that
        // created it. Callers that need an exact version should call
        // `python --version` themselves.
        version: String::new(),
        source: PythonSource::Managed,
    })
}

fn locate_venv_python(venv_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        venv_dir.join("bin").join("python3"),
        venv_dir.join("bin").join("python"),
        venv_dir.join("Scripts").join("python.exe"),
        venv_dir.join("Scripts").join("python3.exe"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

async fn create_venv(base: &ResolvedPython, venv_dir: &Path) -> Result<()> {
    let mut cmd = tokio::process::Command::new(&base.python_bin);
    cmd.arg("-m")
        .arg("venv")
        .arg(venv_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Bound venv bootstrap by the same overall budget as the pip
    // install step. Without this `python -m venv` can hang
    // indefinitely on a stuck interpreter and the parent task has no
    // way to recover (`kill_on_drop` only triggers when the child
    // handle drops, which never happens while we await `output()`).
    let output = tokio::time::timeout(VENV_INSTALL_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            anyhow!(
                "`{} -m venv {}` exceeded {}s timeout",
                base.python_bin.display(),
                venv_dir.display(),
                VENV_INSTALL_TIMEOUT.as_secs()
            )
        })?
        .with_context(|| {
            format!(
                "spawning `{} -m venv {}`",
                base.python_bin.display(),
                venv_dir.display()
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        bail!(
            "`python -m venv` failed (exit={:?}): {}",
            output.status.code(),
            stderr.trim()
        );
    }
    Ok(())
}

async fn pip_install(venv_python: &Path, requirements: &[&str]) -> Result<()> {
    if requirements.is_empty() {
        return Ok(());
    }
    let resolved = ResolvedPython {
        python_bin: venv_python.to_path_buf(),
        version: String::new(),
        source: PythonSource::Managed,
    };
    // Use the standard pip CLI via `python -m pip install <pkgs>` so
    // we don't need to locate `pip3` on PATH or worry about wrapper
    // shebang quirks across managed vs system pythons.
    let mut spec = PythonLaunchSpec::new(std::path::PathBuf::from("-m"));
    spec.args = std::iter::once("pip".to_string())
        .chain(std::iter::once("install".to_string()))
        .chain(std::iter::once("--disable-pip-version-check".to_string()))
        .chain(requirements.iter().map(|r| r.to_string()))
        .collect();

    let output = run_pip_install(&resolved, &spec).await?;
    if output.exit_code != 0 {
        bail!(
            "pip install failed (exit={}): {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

async fn run_pip_install(
    resolved: &ResolvedPython,
    spec: &PythonLaunchSpec,
) -> Result<super::run::PythonRunOutput> {
    // pip install is the heavy-weight first-call path; reuse the
    // generic run helper but with the venv-install timeout instead of
    // the per-tool default.
    //
    // We rebuild spec as a positional one to satisfy
    // spawn_stdio_process's contract: script_path -> first arg after
    // `python`. We pass `-m` as the "script" and `pip install …` as
    // args, matching the `python -m pip install …` form.
    let _ = spec; // placeholder — actually consumed below
    let mut launch = PythonLaunchSpec::new(std::path::PathBuf::from("-m"));
    launch.args = spec.args.clone();
    launch.unbuffered = false;
    // Reuse spawn_stdio_process via the run helper so timeout +
    // stdout/stderr capture are uniform.
    let mut child =
        spawn_stdio_process(resolved, &launch).context("spawning python -m pip install")?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.shutdown().await;
    }
    let output_future = child.wait_with_output();
    let output = match tokio::time::timeout(VENV_INSTALL_TIMEOUT, output_future).await {
        Ok(r) => r.context("waiting on pip install subprocess")?,
        Err(_) => bail!(
            "pip install exceeded {}s timeout",
            VENV_INSTALL_TIMEOUT.as_secs()
        ),
    };
    Ok(super::run::PythonRunOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn render_requirements_lock(requirements: &[&str]) -> String {
    let mut owned: Vec<&str> = requirements.to_vec();
    owned.sort_unstable();
    owned.dedup();
    let mut out = String::new();
    for line in owned {
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_requirements_lock_sorts_and_dedupes() {
        let out = render_requirements_lock(&["python-pptx==1.0.2", "six", "six", "absl-py"]);
        assert_eq!(out, "absl-py\npython-pptx==1.0.2\nsix\n");
    }

    #[test]
    fn validate_venv_name_accepts_simple_names() {
        assert!(validate_venv_name("pptx").is_ok());
        assert!(validate_venv_name("presentation_v1").is_ok());
    }

    #[test]
    fn validate_venv_name_rejects_separators_and_traversal() {
        assert!(validate_venv_name("").is_err());
        assert!(validate_venv_name(".").is_err());
        assert!(validate_venv_name("..").is_err());
        assert!(validate_venv_name("a/b").is_err());
        assert!(validate_venv_name("a\\b").is_err());
    }

    #[test]
    fn resolve_cache_root_honors_configured_dir() {
        let cfg = RuntimePythonConfig {
            cache_dir: "/tmp/custom-py-cache".to_string(),
            ..Default::default()
        };
        assert_eq!(
            resolve_cache_root(&cfg),
            PathBuf::from("/tmp/custom-py-cache")
        );
    }

    #[test]
    fn locate_venv_python_finds_bin_python() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let python = bin.join("python3");
        std::fs::write(&python, b"#!/bin/sh\necho 3.12\n").unwrap();
        assert_eq!(locate_venv_python(temp.path()).as_ref(), Some(&python));
    }

    #[test]
    fn locate_venv_python_returns_none_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        assert!(locate_venv_python(temp.path()).is_none());
    }
}
