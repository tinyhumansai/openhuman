//! One-time provisioning of spaCy into a dedicated virtualenv under the
//! managed Python runtime.
//!
//! The deterministic retriever needs spaCy + a small English model to extract
//! entities from a query. The managed CPython distribution
//! (`runtime_python`) ships bare, so the first call here:
//!   1. resolves a Python ≥ 3.12 via [`PythonBootstrap`],
//!   2. creates an isolated venv (so we never mutate system/site packages),
//!   3. `pip install`s spaCy and downloads `en_core_web_sm`,
//!   4. writes a marker file so subsequent launches skip straight to spawning.
//!
//! All of this is network + filesystem heavy but happens at most once per host
//! (guarded by the marker). Any failure propagates as an error so the caller
//! (`nlp::extract_query_entities`) can fall back to the in-Rust extractor.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use crate::openhuman::config::Config;
use crate::openhuman::runtime_python::PythonBootstrap;

/// Embedded stdio service script, written to disk at provision time so the
/// Python interpreter has a real path to execute.
const SERVICE_PY: &str = include_str!("service.py");

/// Model spaCy downloads / loads. Small English pipeline — fast to load, ~12MB.
pub const SPACY_MODEL: &str = "en_core_web_sm";

/// Timeouts for the one-time install steps. spaCy + model is a few hundred MB
/// of wheels; give pip room on a cold cache / slow link.
const VENV_TIMEOUT: Duration = Duration::from_secs(120);
const PIP_TIMEOUT: Duration = Duration::from_secs(600);

/// A provisioned spaCy runtime: the venv interpreter plus the service script.
#[derive(Debug, Clone)]
pub struct SpacyRuntime {
    /// Python executable inside the dedicated venv (has spaCy + model).
    pub python_bin: PathBuf,
    /// Path to the written `service.py` stdio server.
    pub service_script: PathBuf,
}

/// Ensure spaCy + the model are installed and return a ready-to-spawn runtime.
///
/// Idempotent across calls: once the marker file exists we skip venv creation
/// and pip entirely. Errors here are non-fatal to retrieval — the caller falls
/// back to the regex extractor.
pub async fn ensure_spacy(config: &Config) -> Result<SpacyRuntime> {
    if !config.runtime_python.enabled {
        bail!("runtime_python disabled — cannot provision spaCy");
    }

    let root = nlp_cache_root(config);
    tokio::fs::create_dir_all(&root)
        .await
        .with_context(|| format!("creating nlp cache dir {}", root.display()))?;

    let venv_dir = root.join("spacy-venv");
    let marker = venv_dir.join(".openhuman-spacy-ready");
    let service_script = root.join("service.py");

    // Always (re)write the service script so an upgraded binary ships the
    // latest protocol. Cheap (~3KB) and keeps the on-disk copy authoritative.
    tokio::fs::write(&service_script, SERVICE_PY)
        .await
        .with_context(|| format!("writing spaCy service script {}", service_script.display()))?;

    let venv_python = venv_python_path(&venv_dir);

    if marker.exists() && venv_python.exists() {
        log::debug!(
            "[memory_tree::nlp] spaCy already provisioned at {}",
            venv_dir.display()
        );
        return Ok(SpacyRuntime {
            python_bin: venv_python,
            service_script,
        });
    }

    log::info!(
        "[memory_tree::nlp] provisioning spaCy (one-time): venv={} model={}",
        venv_dir.display(),
        SPACY_MODEL
    );

    // 1. Resolve a base Python interpreter (managed download or system).
    let bootstrap = PythonBootstrap::new(config.runtime_python.clone());
    let base = bootstrap
        .resolve()
        .await
        .context("resolving base python for spaCy venv")?;
    log::debug!(
        "[memory_tree::nlp] base python resolved version={} bin={}",
        base.version,
        base.python_bin.display()
    );

    // 2. Create the venv (idempotent — venv is safe to re-run).
    run_step(
        &base.python_bin,
        &["-m", "venv", &venv_dir.to_string_lossy()],
        VENV_TIMEOUT,
        "create venv",
    )
    .await?;

    if !venv_python.exists() {
        bail!(
            "venv created but interpreter missing at {}",
            venv_python.display()
        );
    }

    // 3. Upgrade pip + install spaCy.
    run_step(
        &venv_python,
        &["-m", "pip", "install", "--upgrade", "pip", "spacy"],
        PIP_TIMEOUT,
        "pip install spacy",
    )
    .await?;

    // 4. Download the model into the venv.
    run_step(
        &venv_python,
        &["-m", "spacy", "download", SPACY_MODEL],
        PIP_TIMEOUT,
        "spacy download model",
    )
    .await?;

    // 5. Marker — provisioning complete.
    tokio::fs::write(&marker, base.version.as_bytes())
        .await
        .with_context(|| format!("writing spaCy ready marker {}", marker.display()))?;

    log::info!("[memory_tree::nlp] spaCy provisioning complete");
    Ok(SpacyRuntime {
        python_bin: venv_python,
        service_script,
    })
}

/// Run one provisioning subprocess, capturing output and surfacing a useful
/// error on non-zero exit or timeout.
async fn run_step(python_bin: &Path, args: &[&str], timeout: Duration, label: &str) -> Result<()> {
    log::debug!(
        "[memory_tree::nlp] step `{label}`: {} {:?}",
        python_bin.display(),
        args
    );
    let mut cmd = Command::new(python_bin);
    cmd.args(args);
    cmd.kill_on_drop(true);

    let output = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(e).with_context(|| format!("spawning step `{label}`")),
        Err(_) => bail!("step `{label}` timed out after {:?}", timeout),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Tail only — pip output is verbose and may include paths but no
        // secrets; cap to keep logs and the error bounded.
        let tail: String = stderr
            .chars()
            .rev()
            .take(800)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        bail!("step `{label}` failed (status {}): {tail}", output.status);
    }
    Ok(())
}

/// Cheap, network-free probe: is spaCy already provisioned on this host?
///
/// Mirrors the early-return guard in [`ensure_spacy`] (marker file + venv
/// interpreter present) without creating directories, writing the service
/// script, or resolving Python. Used by the harness-init orchestrator to mark
/// the spaCy step `Done` instantly on a warm host.
pub fn spacy_provisioned(config: &Config) -> bool {
    let venv_dir = nlp_cache_root(config).join("spacy-venv");
    let marker = venv_dir.join(".openhuman-spacy-ready");
    marker.exists() && venv_python_path(&venv_dir).exists()
}

/// Resolve the venv's python executable across platforms.
fn venv_python_path(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    }
}

/// Cache root for NLP artefacts. Honours `runtime_python.cache_dir` when set
/// (keeps all Python state together), else the user cache dir, else a
/// workspace-relative fallback.
fn nlp_cache_root(config: &Config) -> PathBuf {
    let configured = config.runtime_python.cache_dir.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured).join("memory-nlp");
    }
    if let Some(user_cache) = dirs::cache_dir() {
        return user_cache.join("openhuman").join("memory-nlp");
    }
    config.workspace_dir.join("memory_tree").join("nlp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venv_python_path_is_platform_specific() {
        let p = venv_python_path(Path::new("/tmp/venv"));
        if cfg!(windows) {
            assert!(p.ends_with("Scripts/python.exe") || p.ends_with("Scripts\\python.exe"));
        } else {
            assert_eq!(p, PathBuf::from("/tmp/venv/bin/python"));
        }
    }

    #[test]
    fn cache_root_honours_configured_dir() {
        let mut cfg = Config::default();
        cfg.runtime_python.cache_dir = "/custom/py".to_string();
        assert_eq!(
            nlp_cache_root(&cfg),
            PathBuf::from("/custom/py").join("memory-nlp")
        );
    }
}
