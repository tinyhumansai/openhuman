//! Types describing a loadable module and where its artifact comes from.

use serde::{Deserialize, Serialize};

/// A first-party module this build knows how to load.
///
/// Every field is compiled in rather than discovered: which modules exist, which
/// interfaces they claim, and which bytes are legitimate are decisions that
/// belong to the build, not to whatever a release page happens to serve today.
/// See [`crate::modules::registry`] for why that matters.
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
    /// How to build the module from its vendored gitlink, for local use.
    pub source: SourceSpec,
}

impl ModuleRecord {
    /// The asset for `host_key`, if this release publishes one.
    #[must_use]
    pub fn asset_for(&self, host_key: &str) -> Option<&'static PlatformAsset> {
        self.assets.iter().find(|asset| asset.host_key == host_key)
    }

    /// The part of this record the tinybus resolver consumes: identity, bus
    /// address and the pinned release. The rest (`description`, `load`,
    /// `source`) is host policy.
    #[must_use]
    pub fn release(&self) -> tinybus::module::ReleaseRecord {
        tinybus::module::ReleaseRecord {
            id: self.id,
            bus_name: self.bus_name,
            object_path: self.object_path,
            version: self.version,
            release_url: self.release_url,
            assets: self.assets,
        }
    }
}

/// How to build a module's native library from its gitlink under `vendor/`.
///
/// Used only by `scripts/modules/build.mjs` and the `source` mode, both for
/// local development and local tests. It says nothing about legitimacy: the
/// registry digests in [`PlatformAsset`] are the only pins, and a local build
/// is trusted by the operator through a `modules.toml` the build step writes
/// beside the artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpec {
    /// Directory name under `vendor/` holding the module's repository.
    pub submodule: &'static str,
    /// Path of the module crate's `Cargo.toml`, relative to the submodule.
    pub manifest: &'static str,
    /// Cargo package that produces the `cdylib`.
    pub package: &'static str,
    /// Library file stem Cargo emits, without the platform prefix or suffix.
    pub lib_name: &'static str,
    /// Cargo features to enable on top of the package defaults.
    pub features: &'static [&'static str],
}

impl SourceSpec {
    /// The file name Cargo gives the built library on this platform.
    #[must_use]
    pub fn artifact_filename(&self) -> String {
        if cfg!(target_os = "windows") {
            format!("{}.dll", self.lib_name)
        } else if cfg!(target_os = "macos") {
            format!("lib{}.dylib", self.lib_name)
        } else {
            format!("lib{}.so", self.lib_name)
        }
    }
}

/// One published artifact and the digest that makes it legitimate: the digest
/// is copied verbatim from the release's `checksum.toml`. This is tinybus'
/// own type, so the registry records feed the resolver without a conversion.
pub type PlatformAsset = tinybus::module::ReleaseAsset<'static>;

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

/// Which source actually serves (or would serve) a module.
///
/// Reported in `modules.list`. It can differ from the mode requested: a
/// `linked` request in a binary with no linked factory is served by `release`.
/// Never carries a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleSource {
    /// The registry-pinned, digest-checked GitHub release.
    Release,
    /// The in-process factory from `openhuman-modules-linked`.
    Linked,
    /// A locally built artifact selected through the `source` mode.
    Source,
    /// A local artifact named by a `[[modules.overrides]]` `path`.
    LocalFile,
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
    /// Switched off by `[components]`: never downloaded, loaded or registered.
    Disabled,
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
    /// Source that serves this module under the current configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ModuleSource>,
    /// Why the module is in [`ModuleState::Failed`] or
    /// [`ModuleState::Unsupported`], if it is.
    ///
    /// Never carries a path, a URL with credentials, or any payload: a status
    /// reply is rendered into a UI and copied into bug reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
