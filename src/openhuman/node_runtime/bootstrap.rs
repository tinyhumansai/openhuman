//! Node.js bootstrap orchestrator.
//!
//! Ties the [`resolver`](super::resolver), [`downloader`](super::downloader),
//! and [`extractor`](super::extractor) modules into a single idempotent
//! entry point that callers use at startup (or lazily before the first
//! `node_exec` / `npm_exec` call):
//!
//! ```text
//! NodeBootstrap::new(config) -> resolve() -> ResolvedNode { node_bin, npm_bin, .. }
//! ```
//!
//! The bootstrap is **serialised** through a `tokio::sync::Mutex` so that
//! concurrent callers never race on the download/extract/install pipeline.
//! Once a resolution succeeds the result is memoised — subsequent calls
//! return the cached `ResolvedNode` in O(1).

use anyhow::{bail, Context, Result};
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::downloader::{download_distribution, fetch_shasums, NodeDistribution};
use super::extractor::{atomic_install, extract_distribution};
use super::resolver::{detect_system_node, SystemNode};
use crate::openhuman::config::schema::NodeConfig;

/// Origin of the resolved toolchain — feeds into logging and lets the
/// caller decide whether to expose a "Node was downloaded to …" message in
/// the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSource {
    /// Reused a compatible `node` already on the host `PATH`.
    System,
    /// Downloaded + extracted a managed distribution.
    Managed,
}

/// Fully-resolved Node.js toolchain. Callers should only cache this via the
/// [`NodeBootstrap`] — constructing one by hand bypasses version pinning.
#[derive(Debug, Clone)]
pub struct ResolvedNode {
    /// Directory that should be prepended to `PATH` for child processes so
    /// `node`, `npm`, `npx`, `corepack` resolve to the managed binaries.
    pub bin_dir: PathBuf,
    /// Absolute path to the `node` binary.
    pub node_bin: PathBuf,
    /// Absolute path to the `npm` launcher (shell script on Unix, `.cmd`
    /// shim on Windows). Symlinks on Unix distributions point at a JS file
    /// in `lib/` — invoking through the launcher is the supported contract.
    pub npm_bin: PathBuf,
    /// Version string without the leading `v` (e.g. `"22.11.0"`).
    pub version: String,
    /// Where the toolchain came from.
    pub source: NodeSource,
}

/// Serialised bootstrap entrypoint. Hold one per process (e.g. behind a
/// `OnceCell`) — the internal mutex is what makes concurrent `resolve()`
/// calls safe.
pub struct NodeBootstrap {
    config: NodeConfig,
    workspace_dir: PathBuf,
    client: Client,
    cached: Arc<Mutex<Option<ResolvedNode>>>,
}

impl NodeBootstrap {
    /// Build a new bootstrap. `workspace_dir` is used to derive the default
    /// cache location when `config.cache_dir` is empty.
    pub fn new(config: NodeConfig, workspace_dir: PathBuf, client: Client) -> Self {
        Self {
            config,
            workspace_dir,
            client,
            cached: Arc::new(Mutex::new(None)),
        }
    }

    /// Peek at the memoised [`ResolvedNode`] without triggering a download.
    ///
    /// Returns `Some(..)` only when a previous `resolve()` call succeeded
    /// and the cache lock is currently free. Returns `None` otherwise —
    /// e.g. no resolution has happened yet, or another task holds the
    /// lock doing the initial install. Callers use this for transparent
    /// PATH injection (shell tool) where a blocking wait or a forced
    /// download would change the semantics of unrelated commands.
    pub fn try_cached(&self) -> Option<ResolvedNode> {
        self.cached.try_lock().ok().and_then(|g| g.clone())
    }

    /// Resolve the Node.js toolchain, downloading + extracting a managed
    /// distribution if necessary. Idempotent: the first successful call
    /// memoises the result; later calls return it without further I/O.
    pub async fn resolve(&self) -> Result<ResolvedNode> {
        let mut guard = self.cached.lock().await;
        if let Some(existing) = guard.as_ref() {
            tracing::debug!(
                version = %existing.version,
                source = ?existing.source,
                "[node_runtime::bootstrap] returning cached ResolvedNode"
            );
            return Ok(existing.clone());
        }

        if !self.config.enabled {
            bail!("node runtime is disabled (set node.enabled = true to use skills that require node/npm)");
        }

        if self.config.prefer_system {
            if let Some(system) = detect_system_node(&self.config.version) {
                let resolved = resolve_from_system(system)?;
                *guard = Some(resolved.clone());
                return Ok(resolved);
            }
        }

        let managed = self.install_managed().await?;
        *guard = Some(managed.clone());
        Ok(managed)
    }

    /// Compute the cache root. Uses `config.cache_dir` when set, otherwise
    /// `{workspace}/node-runtime/`.
    fn cache_root(&self) -> PathBuf {
        if self.config.cache_dir.trim().is_empty() {
            self.workspace_dir.join("node-runtime")
        } else {
            PathBuf::from(self.config.cache_dir.trim())
        }
    }

    /// Full install path for the managed distribution. Matches the
    /// archive's top-level folder name so `find_single_top_level` picks the
    /// same directory when re-validating an existing install.
    fn install_dir(&self, dist: &NodeDistribution) -> PathBuf {
        // `archive_name` is e.g. `node-v22.11.0-darwin-arm64.tar.xz`.
        // Strip the extension(s) to get the install folder name.
        let stem = dist
            .archive_name
            .trim_end_matches(".zip")
            .trim_end_matches(".tar.xz")
            .trim_end_matches(".tar")
            .to_string();
        self.cache_root().join(stem)
    }

    /// Full managed-install flow:
    /// 1. Shortcut if an extracted install already exists and has valid
    ///    `node`/`npm` binaries.
    /// 2. Otherwise fetch `SHASUMS256.txt`, pick the matching digest,
    ///    download the archive, extract it, and atomically install.
    async fn install_managed(&self) -> Result<ResolvedNode> {
        let dist = NodeDistribution::for_host(&self.config.version)?;
        let install_dir = self.install_dir(&dist);

        if let Some(resolved) = probe_managed_install(&install_dir, &self.config.version) {
            tracing::info!(
                install_dir = %install_dir.display(),
                "[node_runtime::bootstrap] reusing existing managed install"
            );
            return Ok(resolved);
        }

        tracing::info!(
            version = %dist.version,
            install_dir = %install_dir.display(),
            "[node_runtime::bootstrap] installing managed node"
        );

        let shasums = fetch_shasums(&self.client, &self.config.version).await?;
        let expected = shasums
            .get(&dist.archive_name)
            .cloned()
            .with_context(|| format!("SHASUMS256.txt missing entry for {}", dist.archive_name))?;

        let cache_root = self.cache_root();
        tokio::fs::create_dir_all(&cache_root)
            .await
            .with_context(|| format!("creating cache root {}", cache_root.display()))?;
        let archive_path = cache_root.join(&dist.archive_name);
        download_distribution(&self.client, &dist, &archive_path, &expected).await?;

        // Extract into a scratch folder so a partial extraction never
        // contaminates the cache root; `atomic_install` promotes the
        // inner top-level folder into the final install path.
        let scratch = cache_root.join(format!(".stage-{}", std::process::id()));
        // Wipe any leftover from a previous crashed run.
        let _ = tokio::fs::remove_dir_all(&scratch).await;
        let top_level = extract_distribution(&archive_path, &scratch, dist.is_zip).await?;
        atomic_install(&top_level, &install_dir).await?;
        let _ = tokio::fs::remove_dir_all(&scratch).await;
        let _ = tokio::fs::remove_file(&archive_path).await;

        let bin_dir = managed_bin_dir(&install_dir);
        let version = dist.version.trim_start_matches('v').to_string();
        build_resolved(bin_dir, version, NodeSource::Managed)
    }
}

/// Host-specific bin layout.
///
/// * macOS/Linux: `<install>/bin/{node,npm}`
/// * Windows:     `<install>/{node.exe,npm.cmd}` (no `bin/` subdir in the
///   official zip distributions)
fn managed_bin_dir(install_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        install_dir.to_path_buf()
    } else {
        install_dir.join("bin")
    }
}

/// Build a [`ResolvedNode`] from a bin directory by filling in the
/// platform-specific executable names.
fn build_resolved(bin_dir: PathBuf, version: String, source: NodeSource) -> Result<ResolvedNode> {
    let (node_name, npm_name) = if cfg!(windows) {
        ("node.exe", "npm.cmd")
    } else {
        ("node", "npm")
    };
    let node_bin = bin_dir.join(node_name);
    let npm_bin = bin_dir.join(npm_name);
    if !node_bin.is_file() {
        bail!(
            "resolved node bin missing: {} — install appears corrupted",
            node_bin.display()
        );
    }
    if !npm_bin.exists() {
        tracing::warn!(
            npm_bin = %npm_bin.display(),
            "[node_runtime::bootstrap] npm launcher missing; npm_exec tool will fail until reinstall"
        );
    }
    Ok(ResolvedNode {
        bin_dir,
        node_bin,
        npm_bin,
        version,
        source,
    })
}

/// Wrap a detected system node in a [`ResolvedNode`].
///
/// `detect_system_node` already strips the leading `v` from the probed
/// version, but we re-normalise here so the `ResolvedNode::version`
/// contract (no leading `v`) cannot be violated by any future code path
/// that constructs a `SystemNode` differently.
fn resolve_from_system(system: SystemNode) -> Result<ResolvedNode> {
    let bin_dir = system
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let version = system
        .version
        .trim_start_matches(|c: char| c == 'v' || c == 'V')
        .trim()
        .to_string();
    build_resolved(bin_dir, version, NodeSource::System)
}

/// Check whether `install_dir` already contains a usable managed install
/// for `target_version`. Cheap enough to run on every `resolve()` because
/// it never touches the network — just a few `stat()` calls.
fn probe_managed_install(install_dir: &Path, target_version: &str) -> Option<ResolvedNode> {
    if !install_dir.is_dir() {
        return None;
    }
    let bin_dir = managed_bin_dir(install_dir);
    let version = target_version.trim_start_matches('v').to_string();
    build_resolved(bin_dir, version, NodeSource::Managed).ok()
}
