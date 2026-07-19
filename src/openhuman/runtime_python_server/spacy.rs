use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::openhuman::config::Config;
use crate::openhuman::runtime_python::PythonBootstrap;

pub const SPACY_MODEL: &str = "en_core_web_sm";

/// Packages installed into the bundled spaCy venv.
///
/// `click` is a transitive dependency of spaCy (via `typer`, whose CLI backs
/// `import spacy`), but resolver quirks on Windows have been observed to drop
/// it from the packaged venv, leaving `import spacy` to fail at runtime with
/// `ModuleNotFoundError: No module named 'click'` (GH-4687). Pinning it here as
/// an explicit dependency guarantees it is present in the venv on every
/// platform regardless of transitive resolution.
const SPACY_PIP_PACKAGES: &[&str] = &["spacy", "click"];

/// Build the `pip install` argument vector for provisioning the spaCy venv.
fn spacy_pip_install_args() -> Vec<&'static str> {
    let mut args = vec!["-m", "pip", "install", "--upgrade", "pip"];
    args.extend_from_slice(SPACY_PIP_PACKAGES);
    args
}

/// Filename of the marker dropped in a fully provisioned venv.
const SPACY_READY_MARKER_NAME: &str = ".openhuman-spacy-ready";

/// Schema version recorded on the first line of the ready marker. Bump this
/// whenever the provisioned package set changes so existing venvs are
/// re-provisioned instead of reused. `v2-click` adds the explicit `click` pin
/// (GH-4687): a venv provisioned before that pin carries the marker but may
/// still be missing `click`, so its (v1) marker must NOT satisfy readiness —
/// otherwise `ensure_spacy` short-circuits and the app keeps hitting
/// `ModuleNotFoundError: No module named 'click'` even after updating.
const SPACY_READY_MARKER_VERSION: &str = "v2-click";

const VENV_TIMEOUT: Duration = Duration::from_secs(120);
const PIP_TIMEOUT: Duration = Duration::from_secs(600);

static SPACY_PROVISION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn spacy_provision_lock() -> &'static Mutex<()> {
    SPACY_PROVISION_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone)]
pub struct SpacyRuntime {
    pub python_bin: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacyEntity {
    pub text: String,
    pub label: String,
    #[serde(default)]
    pub start: u32,
    #[serde(default)]
    pub end: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacyResponse {
    #[serde(default)]
    pub entities: Vec<SpacyEntity>,
    #[serde(default)]
    pub nouns: Vec<String>,
}

pub async fn extract(config: &Config, text: &str) -> Result<SpacyResponse> {
    super::server::request_spacy_extract(config, text).await
}

pub async fn ensure_spacy(config: &Config) -> Result<SpacyRuntime> {
    let _guard = spacy_provision_lock().lock().await;
    if !config.runtime_python.enabled {
        bail!("runtime_python disabled — cannot provision spaCy");
    }

    let root = python_server_cache_root(config);
    tokio::fs::create_dir_all(&root).await.with_context(|| {
        format!(
            "creating runtime python server cache dir {}",
            root.display()
        )
    })?;

    let venv_dir = runtime_spacy_venv_dir(config);
    let venv_python = venv_python_path(&venv_dir);

    if spacy_venv_ready(&venv_dir) {
        log::debug!(
            "[runtime_python_server::spacy] spaCy already provisioned at {}",
            venv_dir.display()
        );
        return Ok(SpacyRuntime {
            python_bin: venv_python,
        });
    }

    if let Some(existing_venv) = migrate_or_reuse_legacy_spacy_venv(config, &venv_dir).await? {
        return Ok(SpacyRuntime {
            python_bin: venv_python_path(&existing_venv),
        });
    }

    log::info!(
        "[runtime_python_server::spacy] provisioning spaCy venv={} model={}",
        venv_dir.display(),
        SPACY_MODEL
    );

    let bootstrap = PythonBootstrap::new(config.runtime_python.clone());
    let base = bootstrap
        .resolve()
        .await
        .context("resolving base python for runtime python server spaCy venv")?;
    log::debug!(
        "[runtime_python_server::spacy] base python resolved version={} bin={}",
        base.version,
        base.python_bin.display()
    );

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

    run_step(
        &venv_python,
        &spacy_pip_install_args(),
        PIP_TIMEOUT,
        "pip install spacy",
    )
    .await?;

    run_step(
        &venv_python,
        &["-m", "spacy", "download", SPACY_MODEL],
        PIP_TIMEOUT,
        "spacy download model",
    )
    .await?;

    // First line is the schema version (checked by `spacy_marker_is_current`);
    // the base python version follows for diagnostics.
    let marker = spacy_ready_marker_path(&venv_dir);
    let marker_contents = format!("{SPACY_READY_MARKER_VERSION}\n{}", base.version);
    tokio::fs::write(&marker, marker_contents.as_bytes())
        .await
        .with_context(|| format!("writing spaCy ready marker {}", marker.display()))?;

    log::info!("[runtime_python_server::spacy] spaCy provisioning complete");
    Ok(SpacyRuntime {
        python_bin: venv_python,
    })
}

async fn run_step(python_bin: &Path, args: &[&str], timeout: Duration, label: &str) -> Result<()> {
    log::debug!(
        "[runtime_python_server::spacy] step `{label}`: {} {:?}",
        python_bin.display(),
        args
    );
    let mut cmd = Command::new(python_bin);
    cmd.args(args);
    cmd.kill_on_drop(true);
    // spaCy venv provisioning can re-run mid-session if the venv is missing or
    // its ready marker is stale, so suppress the Windows conhost flash (GH-4814).
    crate::openhuman::inference::local::process_util::apply_no_window(&mut cmd);

    let output = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(error).with_context(|| format!("spawning step `{label}`")),
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

pub fn spacy_provisioned(config: &Config) -> bool {
    spacy_venv_ready(&runtime_spacy_venv_dir(config))
        || legacy_spacy_venv_dirs(config)
            .into_iter()
            .any(|venv_dir| spacy_venv_ready(&venv_dir))
}

pub(crate) fn python_server_cache_root(config: &Config) -> PathBuf {
    let configured = config.runtime_python.cache_dir.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured).join("runtime-python-server");
    }
    if let Some(user_cache) = dirs::cache_dir() {
        return user_cache.join("openhuman").join("runtime-python-server");
    }
    config.workspace_dir.join("runtime_python_server")
}

fn runtime_spacy_venv_dir(config: &Config) -> PathBuf {
    python_server_cache_root(config).join("spacy-venv")
}

async fn migrate_or_reuse_legacy_spacy_venv(
    config: &Config,
    target_venv: &Path,
) -> Result<Option<PathBuf>> {
    for legacy_venv in legacy_spacy_venv_dirs(config) {
        if legacy_venv == target_venv || !spacy_venv_ready(&legacy_venv) {
            continue;
        }

        if !target_venv.exists() {
            if let Some(parent) = target_venv.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("creating spaCy venv parent {}", parent.display()))?;
            }
            match tokio::fs::rename(&legacy_venv, target_venv).await {
                Ok(()) => {
                    log::info!(
                        "[runtime_python_server::spacy] migrated legacy spaCy venv {} -> {}",
                        legacy_venv.display(),
                        target_venv.display()
                    );
                    return Ok(Some(target_venv.to_path_buf()));
                }
                Err(error) => {
                    log::warn!(
                        "[runtime_python_server::spacy] could not migrate legacy spaCy venv {} -> {}; reusing legacy path: {error}",
                        legacy_venv.display(),
                        target_venv.display()
                    );
                    return Ok(Some(legacy_venv));
                }
            }
        }

        log::info!(
            "[runtime_python_server::spacy] reusing legacy spaCy venv {} because target {} is not ready",
            legacy_venv.display(),
            target_venv.display()
        );
        return Ok(Some(legacy_venv));
    }

    Ok(None)
}

fn legacy_spacy_venv_dirs(config: &Config) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let configured = config.runtime_python.cache_dir.trim();
    if !configured.is_empty() {
        roots.push(PathBuf::from(configured).join("memory-nlp"));
    } else if let Some(user_cache) = dirs::cache_dir() {
        roots.push(user_cache.join("openhuman").join("memory-nlp"));
    }
    roots.push(config.workspace_dir.join("memory_tree").join("nlp"));
    roots
        .into_iter()
        .map(|root| root.join("spacy-venv"))
        .collect()
}

fn spacy_ready_marker_path(venv_dir: &Path) -> PathBuf {
    venv_dir.join(SPACY_READY_MARKER_NAME)
}

/// True when the venv's ready marker exists AND records the current schema
/// version. A missing marker, an unreadable marker, or a marker from an older
/// schema (e.g. a pre-GH-4687 venv that may lack `click`) all read as "not
/// ready" so the venv is re-provisioned with the current package set rather
/// than reused.
fn spacy_marker_is_current(venv_dir: &Path) -> bool {
    match std::fs::read_to_string(spacy_ready_marker_path(venv_dir)) {
        Ok(contents) => contents.lines().next().map(str::trim) == Some(SPACY_READY_MARKER_VERSION),
        Err(_) => false,
    }
}

fn spacy_venv_ready(venv_dir: &Path) -> bool {
    spacy_marker_is_current(venv_dir) && venv_python_path(venv_dir).exists()
}

fn venv_python_path(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_root_honours_runtime_python_cache_dir() {
        let mut config = Config::default();
        config.runtime_python.cache_dir = "/tmp/openhuman-python".to_string();
        assert_eq!(
            python_server_cache_root(&config),
            PathBuf::from("/tmp/openhuman-python").join("runtime-python-server")
        );
    }

    #[test]
    fn legacy_configured_cache_is_considered_provisioned() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.runtime_python.cache_dir = temp.path().to_string_lossy().to_string();
        let legacy_venv = temp.path().join("memory-nlp").join("spacy-venv");
        std::fs::create_dir_all(legacy_venv.join(if cfg!(windows) { "Scripts" } else { "bin" }))
            .unwrap();
        std::fs::write(
            spacy_ready_marker_path(&legacy_venv),
            format!("{SPACY_READY_MARKER_VERSION}\n3.11.0"),
        )
        .unwrap();
        std::fs::write(venv_python_path(&legacy_venv), "").unwrap();

        assert!(spacy_provisioned(&config));
    }

    #[test]
    fn stale_marker_venv_is_not_ready_so_it_reprovisions() {
        // Regression for GH-4687: a venv provisioned before the `click` pin
        // carries the ready marker but may be missing `click`. Its old-schema
        // marker must not satisfy readiness, so `ensure_spacy` re-provisions
        // (re-running pip with the `click` pin) instead of short-circuiting and
        // continuing to fail with `ModuleNotFoundError: click`.
        let temp = tempfile::tempdir().unwrap();
        let venv = temp.path().join("spacy-venv");
        std::fs::create_dir_all(venv.join(if cfg!(windows) { "Scripts" } else { "bin" })).unwrap();
        std::fs::write(venv_python_path(&venv), "").unwrap();

        // Pre-#4687 marker wrote only the python version — no schema tag.
        std::fs::write(spacy_ready_marker_path(&venv), "3.11.5").unwrap();
        assert!(
            !spacy_venv_ready(&venv),
            "stale-marker venv must be re-provisioned, not treated as ready"
        );

        // Current-schema marker satisfies readiness.
        std::fs::write(
            spacy_ready_marker_path(&venv),
            format!("{SPACY_READY_MARKER_VERSION}\n3.11.5"),
        )
        .unwrap();
        assert!(
            spacy_venv_ready(&venv),
            "current-schema marker venv is ready"
        );
    }

    #[test]
    fn pip_install_args_include_click_dependency() {
        // Regression for GH-4687: `click` must be an explicit venv dependency so
        // it is never dropped from the packaged runtime on Windows, where its
        // absence breaks `import spacy` with `ModuleNotFoundError: click`.
        let args = spacy_pip_install_args();
        assert!(args.contains(&"click"), "click must be installed: {args:?}");
        assert!(args.contains(&"spacy"), "spacy must be installed: {args:?}");
        assert_eq!(&args[..5], &["-m", "pip", "install", "--upgrade", "pip"]);
    }

    #[test]
    fn spacy_response_parses() {
        let response: SpacyResponse = serde_json::from_str(
            r#"{"entities":[{"text":"Alice","label":"PERSON","start":0,"end":5}],"nouns":["migration"]}"#,
        )
        .unwrap();
        assert_eq!(response.entities[0].label, "PERSON");
        assert_eq!(response.nouns, vec!["migration"]);
    }

    // Exercises the venv provisioning step path (including the GH-4814
    // CREATE_NO_WINDOW hook, a no-op off Windows) with a trivial binary so it
    // stays covered without a real python toolchain.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_step_runs_a_trivial_binary() {
        run_step(
            Path::new("/bin/echo"),
            &["ok"],
            Duration::from_secs(30),
            "echo smoke",
        )
        .await
        .expect("echo step succeeds");
    }
}
