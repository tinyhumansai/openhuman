//! Kompress backend — TokenJuice ML plain-text compressor (ModernBERT/torch).
//!
//! Provisions torch + transformers into the runtime python server's venv and
//! pre-downloads the model so the long-lived server can load it offline at
//! startup. The actual compression runs inside the shared `server.py` and is
//! reached via [`request_kompress`] (→ `server::request("kompress.compress")`).
//!
//! Two provisioning entry points so the single-venv server can host Kompress
//! alongside (or instead of) spaCy:
//! - [`ensure_kompress`] creates a dedicated `kompress-venv` when Kompress is
//!   the only heavy backend.
//! - [`install_into`] adds torch + transformers to an *existing* venv (e.g. the
//!   spaCy venv) when both backends are enabled and must share one interpreter.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::OnceCell;

use crate::openhuman::config::Config;
use crate::openhuman::runtime_python::PythonBootstrap;

use super::spacy::python_server_cache_root;

const VENV_TIMEOUT: Duration = Duration::from_secs(120);
/// torch + transformers wheels are large; allow a generous one-time window.
const PIP_TIMEOUT: Duration = Duration::from_secs(1800);
const MODEL_TIMEOUT: Duration = Duration::from_secs(1800);

static PROVISION_LOCK: OnceCell<tokio::sync::Mutex<()>> = OnceCell::const_new();

async fn provision_lock() -> &'static tokio::sync::Mutex<()> {
    PROVISION_LOCK
        .get_or_init(|| async { tokio::sync::Mutex::new(()) })
        .await
}

/// A provisioned Kompress runtime: the venv interpreter with torch+transformers
/// and the HuggingFace cache holding the (pre-downloaded) model weights.
#[derive(Debug, Clone)]
pub struct KompressRuntime {
    pub python_bin: PathBuf,
    pub hf_home: PathBuf,
}

/// Result of one `kompress.compress` call.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KompressResponse {
    #[serde(default)]
    pub compressed_text: String,
    #[serde(default)]
    pub input_chars: usize,
    #[serde(default)]
    pub output_chars: usize,
}

/// Compress `text` with the Kompress backend over the shared runtime server.
pub async fn request_kompress(config: &Config, text: &str) -> Result<KompressResponse> {
    super::server::request_kompress_compress(config, text).await
}

/// HF cache directory for Kompress model weights (kept under the server cache).
pub(crate) fn hf_home(config: &Config) -> PathBuf {
    python_server_cache_root(config).join("kompress-hf")
}

fn kompress_venv_dir(config: &Config) -> PathBuf {
    python_server_cache_root(config).join("kompress-venv")
}

fn venv_python_path(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    }
}

fn marker_path(venv_dir: &Path, model_id: &str) -> PathBuf {
    let safe_model: String = model_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    venv_dir.join(format!(".openhuman-kompress-ready-{safe_model}"))
}

/// Cheap, network-free probe: are torch + transformers + the model provisioned?
pub fn kompress_provisioned(config: &Config) -> bool {
    let venv = kompress_venv_dir(config);
    marker_path(&venv, &config.tokenjuice.ml_model_id).exists() && venv_python_path(&venv).exists()
}

/// Ensure a dedicated Kompress venv exists (torch + transformers + model).
pub async fn ensure_kompress(config: &Config) -> Result<KompressRuntime> {
    let _guard = provision_lock().await.lock().await;
    if !config.runtime_python.enabled {
        bail!("runtime_python disabled — cannot provision Kompress");
    }
    if !config.tokenjuice.ml_compression_enabled {
        bail!("tokenjuice.ml_compression_enabled is false");
    }

    let venv_dir = kompress_venv_dir(config);
    let venv_python = venv_python_path(&venv_dir);
    let hf = hf_home(config);

    if marker_path(&venv_dir, &config.tokenjuice.ml_model_id).exists() && venv_python.exists() {
        return Ok(KompressRuntime {
            python_bin: venv_python,
            hf_home: hf,
        });
    }

    tokio::fs::create_dir_all(&hf)
        .await
        .with_context(|| format!("creating kompress hf home {}", hf.display()))?;

    log::info!(
        "[runtime_python_server::kompress] provisioning venv={} model={}",
        venv_dir.display(),
        config.tokenjuice.ml_model_id
    );

    let base = PythonBootstrap::new(config.runtime_python.clone())
        .resolve()
        .await
        .context("resolving base python for kompress venv")?;

    run_step(
        &base.python_bin,
        &["-m", "venv", &venv_dir.to_string_lossy()],
        VENV_TIMEOUT,
        &hf,
        "create venv",
    )
    .await?;
    if !venv_python.exists() {
        bail!(
            "venv created but interpreter missing at {}",
            venv_python.display()
        );
    }

    install_deps_and_model(&venv_python, &hf, &config.tokenjuice.ml_model_id).await?;

    tokio::fs::write(
        marker_path(&venv_dir, &config.tokenjuice.ml_model_id),
        base.version.as_bytes(),
    )
    .await
    .with_context(|| "writing kompress ready marker")?;

    log::info!("[runtime_python_server::kompress] provisioning complete");
    Ok(KompressRuntime {
        python_bin: venv_python,
        hf_home: hf,
    })
}

/// Install torch + transformers + the model into an *existing* venv (shared with
/// another backend, e.g. spaCy). Idempotent — a marker next to the interpreter
/// records completion so repeat launches skip the heavy step.
pub async fn install_into(config: &Config, venv_python: &Path) -> Result<PathBuf> {
    let _guard = provision_lock().await.lock().await;
    let hf = hf_home(config);
    let shared_marker = venv_python
        .parent()
        .map(|d| marker_path(d, &config.tokenjuice.ml_model_id))
        .unwrap_or_else(|| marker_path(Path::new("."), &config.tokenjuice.ml_model_id));
    if shared_marker.exists() {
        return Ok(hf);
    }
    tokio::fs::create_dir_all(&hf)
        .await
        .with_context(|| format!("creating kompress hf home {}", hf.display()))?;
    log::info!(
        "[runtime_python_server::kompress] installing torch+transformers into shared venv {}",
        venv_python.display()
    );
    install_deps_and_model(venv_python, &hf, &config.tokenjuice.ml_model_id).await?;
    let _ = tokio::fs::write(&shared_marker, config.tokenjuice.ml_model_id.as_bytes()).await;
    Ok(hf)
}

/// pip-install torch (CPU wheel) + transformers, then pre-download the model so
/// the long-lived server can load it offline.
async fn install_deps_and_model(venv_python: &Path, hf: &Path, model_id: &str) -> Result<()> {
    run_step(
        venv_python,
        &["-m", "pip", "install", "--upgrade", "pip"],
        PIP_TIMEOUT,
        hf,
        "pip upgrade",
    )
    .await?;
    run_step(
        venv_python,
        &[
            "-m",
            "pip",
            "install",
            "--index-url",
            "https://download.pytorch.org/whl/cpu",
            "torch",
        ],
        PIP_TIMEOUT,
        hf,
        "pip install torch (cpu)",
    )
    .await?;
    run_step(
        venv_python,
        &["-m", "pip", "install", "transformers", "tokenizers"],
        PIP_TIMEOUT,
        hf,
        "pip install transformers",
    )
    .await?;
    // Pre-download the model into the HF cache so server startup loads offline.
    let preload = format!(
        "from transformers import AutoModel, AutoTokenizer; \
         AutoTokenizer.from_pretrained('{model_id}'); AutoModel.from_pretrained('{model_id}')"
    );
    run_step(
        venv_python,
        &["-c", &preload],
        MODEL_TIMEOUT,
        hf,
        "preload model",
    )
    .await?;
    Ok(())
}

async fn run_step(
    python_bin: &Path,
    args: &[&str],
    timeout: Duration,
    hf_home: &Path,
    label: &str,
) -> Result<()> {
    log::debug!(
        "[runtime_python_server::kompress] step `{label}`: {} {:?}",
        python_bin.display(),
        args
    );
    let mut cmd = Command::new(python_bin);
    cmd.args(args);
    cmd.env("HF_HOME", hf_home);
    cmd.env("HF_HUB_DISABLE_TELEMETRY", "1");
    cmd.kill_on_drop(true);

    let output = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(e).with_context(|| format!("spawning step `{label}`")),
        Err(_) => bail!("step `{label}` timed out after {:?}", timeout),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
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
    fn provisioned_false_on_clean_config() {
        let mut config = Config::default();
        config.runtime_python.cache_dir = "/nonexistent/tj-test".to_string();
        assert!(!kompress_provisioned(&config));
    }

    #[test]
    fn ready_marker_is_model_specific_and_path_safe() {
        let venv = Path::new("/tmp/openhuman-kompress-test-venv");
        let first = marker_path(venv, "answerdotai/ModernBERT-base");
        let second = marker_path(venv, "other/model");

        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(venv));
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("answerdotai_ModernBERT-base"));
    }
}
