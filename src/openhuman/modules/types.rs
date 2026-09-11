//! Types describing a loadable module and where its artifact comes from.

use serde::{Deserialize, Serialize};

/// A first-party module this build knows how to load.
///
/// Every field is compiled in rather than discovered: which modules exist, which
/// interfaces they claim, and which bytes are legitimate are decisions that
/// belong to the build, not to whatever a release page happens to serve today.
/// See [`crate::openhuman::modules::registry`] for why that matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleRecord {
    /// Stable identifier used by config, RPC, and `ensure_loaded`.
    pub id: &'static str,
    /// Human-readable summary for `modules.list`.
    pub description: &'static str,
    /// Well-known bus name the module claims once it is serving.
    pub bus_name: &'static str,
    /// Object path the module serves its interface at.
    pub object_path: &'static str,
    /// Release version, matching the tag the assets come from.
    pub version: &'static str,
    /// GitHub release tag URL the artifacts are published under.
    pub release_url: &'static str,
    /// Per-host artifacts, in the order [`super::platform`] prefers them.
    pub assets: &'static [PlatformAsset],
    /// When the module is loaded.
    pub load: LoadPolicy,
}

impl ModuleRecord {
    /// The asset for `host_key`, if this release publishes one.
    #[must_use]
    pub fn asset_for(&self, host_key: &str) -> Option<&'static PlatformAsset> {
        self.assets.iter().find(|asset| asset.host_key == host_key)
    }
}

/// One published artifact and the digest that makes it legitimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformAsset {
    /// Host identifier this artifact targets, e.g. `ubuntu-24.04-x86_64`.
    pub host_key: &'static str,
    /// Exact release asset name to download.
    pub archive: &'static str,
    /// Lowercase hex SHA-256 of the archive, taken from the release manifest.
    pub sha256: &'static str,
}

/// When a module is loaded into the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPolicy {
    /// On first use.
    ///
    /// The right default for a codec: a user who never asks for a document
    /// should not pay a download, a `dlopen`, or the resident cost of a library
    /// that is never unloaded.
    Lazy,
    /// At boot, before the first request.
    ///
    /// For a module whose absence would change behaviour rather than just delay
    /// it — one that has to be serving before something else decides what it can
    /// offer.
    Eager,
}

/// Where a module's artifact should come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModuleSource {
    /// The registry-pinned GitHub release for this build.
    Pinned,
    /// A local artifact, for developing a module against a live core.
    LocalFile {
        /// Absolute path to the platform library.
        path: String,
    },
}

/// Lifecycle of a module as this host sees it.
///
/// Deliberately coarser than tinybus's own lifecycle: a caller of `modules.list`
/// wants to know whether it can use the thing, not which admission step it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleState {
    /// Known to this build, not loaded yet.
    Available,
    /// Being downloaded, verified, or initialised.
    Loading,
    /// Serving its interface.
    Ready,
    /// Cannot be loaded in this process, and will not be retried.
    ///
    /// Terminal by tinybus's design: a refused or faulted module keeps its
    /// library mapped, so retrying cannot produce a different outcome without a
    /// restart.
    Failed,
    /// No artifact is published for this host.
    Unsupported,
}

/// A module's status as reported over RPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleStatus {
    /// Registry identifier.
    pub id: String,
    /// Human-readable summary.
    pub description: String,
    /// Pinned release version.
    pub version: String,
    /// Well-known bus name.
    pub bus_name: String,
    /// Current lifecycle state.
    pub state: ModuleState,
    /// Why the module is in [`ModuleState::Failed`] or
    /// [`ModuleState::Unsupported`], if it is.
    ///
    /// Never carries a path, a URL with credentials, or any payload: a status
    /// reply is rendered into a UI and copied into bug reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
