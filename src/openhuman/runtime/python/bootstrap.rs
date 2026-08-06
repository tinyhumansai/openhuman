//! Python bootstrap orchestrator.
//!
//! Resolves a managed standalone CPython distribution by default, with an
//! optional system-Python override for development.

use anyhow::{bail, Context, Result};
use reqwest::Client;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::downloader::{download_distribution, select_distribution};
use super::extractor::{atomic_install, extract_distribution};
use super::resolver::{detect_system_python, SystemPython};
use crate::openhuman::config::schema::RuntimePythonConfig;

/// Origin of the resolved interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonSource {
    /// Reused a compatible Python already available on the host.
    System,
    /// Reserved for a future managed CPython distribution.
    Managed,
}

/// Fully-resolved Python interpreter.
#[derive(Debug, Clone)]
pub struct ResolvedPython {
    /// Directory that should be prepended to `PATH` for child processes so
    /// `python`, `python3`, `pip`, and `pip3` resolve to the same toolchain.
    pub bin_dir: std::path::PathBuf,
    /// Absolute path to the Python executable.
    pub python_bin: std::path::PathBuf,
    /// Normalized interpreter version, e.g. `3.12.4`.
    pub version: String,
    /// Where the interpreter came from.
    pub source: PythonSource,
}

/// Serialised bootstrap entrypoint for Python runtime resolution.
pub struct PythonBootstrap {
    config: RuntimePythonConfig,
    client: Client,
    cached: Arc<Mutex<Option<ResolvedPython>>>,
}

impl PythonBootstrap {
    pub fn new(config: RuntimePythonConfig) -> Self {
        Self::new_with_client(config, Client::new())
    }

    pub(crate) fn new_with_client(config: RuntimePythonConfig, client: Client) -> Self {
        Self {
            config,
            client,
            cached: Arc::new(Mutex::new(None)),
        }
    }

    /// Peek at the memoized interpreter without triggering a probe.
    pub fn try_cached(&self) -> Option<ResolvedPython> {
        self.cached.try_lock().ok().and_then(|g| g.clone())
    }

    /// Resolve a Python 3.12+ interpreter. The first successful result is
    /// memoized for subsequent callers.
    pub async fn resolve(&self) -> Result<ResolvedPython> {
        let mut guard = self.cached.lock().await;
        if let Some(existing) = guard.as_ref() {
            tracing::debug!(
                version = %existing.version,
                source = ?existing.source,
                "[runtime_python::bootstrap] returning cached ResolvedPython"
            );
            return Ok(existing.clone());
        }

        if !self.config.enabled {
            bail!(
                "runtime_python is disabled (set runtime_python.enabled = true to use Python-backed integrations)"
            );
        }

        if self.config.prefer_system {
            if let Some(system) = detect_system_python(
                &self.config.minimum_version,
                empty_to_none(&self.config.preferred_command),
            ) {
                let resolved = resolve_from_system(system);
                *guard = Some(resolved.clone());
                return Ok(resolved);
            }
        }

        let managed = self
            .install_managed_from_api(super::downloader::RELEASES_API)
            .await?;
        *guard = Some(managed.clone());
        Ok(managed)
    }

    /// Durable, **non-downloading** readiness probe.
    ///
    /// Unlike [`try_cached`], whose state is process-local and therefore empty
    /// after every app restart, this inspects the host itself: a compatible
    /// system interpreter (when `prefer_system`) or an already-extracted managed
    /// distribution under the cache root. It performs only cheap `stat`/version
    /// checks and **never** touches the network, so it is safe to call as a
    /// warm-start guard before deciding whether to surface a visible
    /// provisioning run. A hit is memoised into the same cache `resolve()` uses.
    ///
    /// Returns `Some(..)` when Python is already provisioned on disk, `None`
    /// when a genuine download/install is still required (or the runtime is
    /// disabled — callers treat that as "nothing to provision" separately).
    pub async fn probe_installed(&self) -> Option<ResolvedPython> {
        if let Some(existing) = self.try_cached() {
            return Some(existing);
        }
        if !self.config.enabled {
            return None;
        }
        if self.config.prefer_system {
            if let Some(system) = detect_system_python(
                &self.config.minimum_version,
                empty_to_none(&self.config.preferred_command),
            ) {
                let resolved = resolve_from_system(system);
                tracing::debug!(
                    version = %resolved.version,
                    "[runtime_python::bootstrap] durable probe found system python"
                );
                *self.cached.lock().await = Some(resolved.clone());
                return Some(resolved);
            }
        }
        if let Some(resolved) = probe_any_managed_install(&self.cache_root()) {
            tracing::debug!(
                version = %resolved.version,
                "[runtime_python::bootstrap] durable probe found managed python on disk"
            );
            *self.cached.lock().await = Some(resolved.clone());
            return Some(resolved);
        }
        tracing::debug!(
            "[runtime_python::bootstrap] durable probe found no installed python (provisioning required)"
        );
        None
    }

    /// Build a preconfigured child-process launcher for stdio-oriented Python
    /// workloads such as MCP servers.
    pub async fn spawn_stdio(
        &self,
        spec: &crate::openhuman::runtime::python::process::PythonLaunchSpec,
    ) -> Result<tokio::process::Child> {
        let resolved = self.resolve().await?;
        crate::openhuman::runtime::python::process::spawn_stdio_process(&resolved, spec)
    }
}

impl PythonBootstrap {
    async fn install_managed(&self) -> Result<ResolvedPython> {
        self.install_managed_from_api(super::downloader::RELEASES_API)
            .await
    }

    async fn install_managed_from_api(&self, releases_api_base: &str) -> Result<ResolvedPython> {
        let cache_root = self.cache_root();
        tokio::fs::create_dir_all(&cache_root)
            .await
            .with_context(|| format!("creating python runtime cache {}", cache_root.display()))?;

        let release = super::downloader::fetch_release_metadata_from_base(
            &self.client,
            releases_api_base,
            empty_to_none(&self.config.managed_release_tag),
        )
        .await?;
        let dist = select_distribution(
            &release,
            &self.config.minimum_version,
            &self.config.maximum_version,
        )?;
        let install_dir = cache_root.join(dist.install_dir_name());
        let _install_lock = acquire_install_lock(&install_dir).await?;

        if let Some(existing) = probe_managed_install(&install_dir) {
            tracing::info!(
                install_dir = %install_dir.display(),
                version = %existing.version,
                "[runtime_python::bootstrap] reusing existing managed python install"
            );
            return Ok(existing);
        }

        tracing::info!(
            asset = %dist.asset_name,
            release = %release.tag_name,
            install_dir = %install_dir.display(),
            "[runtime_python::bootstrap] installing managed python"
        );

        let archive_path = cache_root.join(&dist.asset_name);
        download_distribution(&self.client, &dist, &archive_path).await?;

        let scratch = cache_root.join(format!(
            ".stage-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = tokio::fs::remove_dir_all(&scratch).await;
        let top_level = extract_distribution(&archive_path, &scratch).await?;
        atomic_install(&top_level, &install_dir).await?;
        let _ = tokio::fs::remove_dir_all(&scratch).await;
        let _ = tokio::fs::remove_file(&archive_path).await;

        probe_managed_install(&install_dir).with_context(|| {
            format!(
                "managed python install completed but no interpreter was found under {}",
                install_dir.display()
            )
        })
    }

    fn cache_root(&self) -> PathBuf {
        let configured = self.config.cache_dir.trim();
        if !configured.is_empty() {
            return PathBuf::from(configured);
        }
        if let Some(user_cache) = dirs::cache_dir() {
            return user_cache.join("openhuman").join("runtime-python");
        }
        PathBuf::from(".openhuman").join("runtime-python")
    }
}

fn resolve_from_system(system: SystemPython) -> ResolvedPython {
    tracing::info!(
        path = %system.path.display(),
        version = %system.version,
        "[runtime_python::bootstrap] reusing compatible system python"
    );
    ResolvedPython {
        bin_dir: python_bin_dir(&system.path),
        python_bin: system.path,
        version: system.version,
        source: PythonSource::System,
    }
}

fn empty_to_none(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Scan the managed cache root for an already-extracted CPython install,
/// probing each immediate subdirectory without any network access. Returns the
/// first usable interpreter found. Used by the durable readiness probe so a
/// warm restart detects a prior managed install without re-downloading — the
/// exact install-dir name is derived from network release metadata during
/// install, so on a cold cache we cannot reconstruct it offline and must scan.
fn probe_any_managed_install(cache_root: &Path) -> Option<ResolvedPython> {
    let entries = std::fs::read_dir(cache_root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        // Reject symlinked cache entries before probing: `path.is_dir()` follows
        // symlinks, so a link planted under `cache_root` could point the durable
        // probe at a directory *outside* the cache root and have it reused as a
        // trusted managed install. `entry.file_type()` reports the link itself
        // (it does not follow), so skip anything symlinked; treat an unknown type
        // as untrusted too. (The Node bootstrap gets equivalent protection from
        // its `canonicalize` + `starts_with(cache_root)` guard.)
        match entry.file_type() {
            Ok(ft) if ft.is_symlink() => continue,
            Ok(_) => {}
            Err(_) => continue,
        }
        if !path.is_dir() {
            continue;
        }
        // Skip transient staging dirs (`.stage-<pid>-<uuid>`) left mid-install.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".stage-"))
        {
            continue;
        }
        if let Some(resolved) = probe_managed_install(&path) {
            return Some(resolved);
        }
    }
    None
}

fn probe_managed_install(install_dir: &Path) -> Option<ResolvedPython> {
    let python_bin = find_python_binary(install_dir)?;
    let version = super::resolver::probe_python_version_public(&python_bin)?;
    let version_info = super::resolver::parse_python_version(&version)?;
    Some(ResolvedPython {
        bin_dir: python_bin_dir(&python_bin),
        python_bin,
        version: version_info.display(),
        source: PythonSource::Managed,
    })
}

fn python_bin_dir(python_bin: &Path) -> PathBuf {
    python_bin
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn find_python_binary(install_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        install_dir.join("bin").join("python3.12"),
        install_dir.join("bin").join("python3"),
        install_dir.join("bin").join("python"),
        install_dir.join("python.exe"),
        install_dir.join("python3.12.exe"),
        install_dir.join("python3.exe"),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    for entry in walkdir::WalkDir::new(install_dir).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if matches!(
            name,
            "python" | "python3" | "python3.12" | "python.exe" | "python3.exe" | "python3.12.exe"
        ) {
            return Some(path.to_path_buf());
        }
    }
    None
}

async fn acquire_install_lock(install_dir: &Path) -> Result<std::fs::File> {
    let lock_path = install_dir.with_extension("lock");
    if let Some(parent) = lock_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating lock parent {}", parent.display()))?;
    }

    let lock_path_for_task = lock_path.clone();
    tokio::task::spawn_blocking(move || -> Result<std::fs::File> {
        use fs2::FileExt;

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path_for_task)
            .with_context(|| format!("opening install lock {}", lock_path_for_task.display()))?;
        file.lock_exclusive()
            .with_context(|| format!("locking install target {}", lock_path_for_task.display()))?;
        Ok(file)
    })
    .await
    .context("join failure while acquiring runtime_python install lock")?
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
