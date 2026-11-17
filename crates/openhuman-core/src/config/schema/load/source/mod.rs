//! Where a config's TOML text lives: the [`ConfigSource`] port.
//!
//! The loader and `Config::save` work on TOML *text*; this port is the only
//! thing that knows where that text is kept, so the codec, the migrations, the
//! secret encryption and the corruption recovery are one code path for every
//! deployment.
//!
//! | Source | Where | Used |
//! | --- | --- | --- |
//! | [`FileConfigSource`] | `config.toml`, replaced atomically (`atomic_commit`) with a `.bak` | desktop, CLI, and always at boot |
//! | [`DocumentConfigSource`] | the `config/{scope}` document on the storage backend | a shared (multi-tenant) backend, for saves and snapshot reloads |
//!
//! # Bootstrap config never comes from storage
//!
//! The `[storage]` table decides which backend exists, so it cannot be read
//! from one (and its URL can carry a database password). [`BOOTSTRAP_TABLES`]
//! are therefore stripped before a document is written and re-applied from the
//! file on every document read. A process boots before its backend is installed,
//! so its first load (`load_or_init`) normally reads the file; nothing
//! enforces that ordering.
//!
//! # Why text, not a JSON tree
//!
//! A document holds the TOML body as one string field. That is lossless
//! (a JSON tree would not round-trip every TOML value), keeps one parser, and
//! keeps the file hand-editable on desktop. The desktop file source keeps
//! `atomic_commit` and its `.bak`, `0600` and fsync guarantees, which the file
//! driver's blob API does not offer.

mod document;
mod file;
pub(crate) mod storage_schema;

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;

pub(crate) use document::DocumentConfigSource;
pub(crate) use file::FileConfigSource;

/// Top-level TOML tables that are bootstrap configuration: always read from
/// the file or environment, never stored in (or read from) a storage backend.
pub(crate) const BOOTSTRAP_TABLES: &[&str] = &["storage"];

/// What a source read returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigRead {
    /// The TOML text. Empty when the content was unreadable and recovery found
    /// nothing (the caller then falls back to defaults).
    pub contents: String,
    /// The source recovered from corruption (a `.bak`, or defaults).
    pub recovered: bool,
    /// The text is a stored config document's body. `false` for a file,
    /// including the file a document source falls back to when the scope has
    /// no document yet: that text keeps the file's corruption recovery.
    pub from_document: bool,
    pub document_version: Option<tinystoragedrivers::Version>,
    pub source_anchor: ConfigSourceAnchor,
}

/// A retained identity for the document backend that owns a snapshot.
#[derive(Clone)]
pub(crate) enum DocumentBackendIdentity {
    Installed(std::sync::Arc<dyn crate::storage::StorageBackend>),
    Test(std::sync::Arc<()>),
}

impl std::fmt::Debug for DocumentBackendIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DocumentBackendIdentity(..)")
    }
}

impl PartialEq for DocumentBackendIdentity {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Installed(left), Self::Installed(right)) => std::sync::Arc::ptr_eq(left, right),
            (Self::Test(left), Self::Test(right)) => std::sync::Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl Eq for DocumentBackendIdentity {}

/// Identity of the exact persistence target a config snapshot was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigSourceAnchor {
    File(std::path::PathBuf),
    Document {
        backend: DocumentBackendIdentity,
        scope: String,
        id: String,
    },
}

pub(crate) fn normalized_path(path: &Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// A place a config's TOML text is read from and written to.
#[async_trait]
pub(crate) trait ConfigSource: Send + Sync {
    /// A short, stable name for logs: `file` or `document`.
    fn label(&self) -> &'static str;
    fn anchor(&self) -> ConfigSourceAnchor;

    /// Whether the source holds a config at all.
    ///
    /// # Errors
    ///
    /// A backend failure. It is never folded into "absent": a transient
    /// outage must not make the loader fall back to defaults.
    async fn exists(&self) -> Result<bool>;

    /// Whether the source encrypts the whole body itself, so `Config::save`
    /// must not also encrypt individual fields under the process-local key
    /// (which another node could not decrypt).
    fn encrypts_body(&self) -> bool {
        false
    }

    /// Read the config text, recovering from corruption where the source can.
    async fn read(&self) -> Result<ConfigRead>;

    /// Replace the config text.
    ///
    /// On a document source, this uses the handle's last successfully observed
    /// revision; a handle that has not observed a document (or observed its
    /// absence) is create-only. This is a last-observation rule for the source
    /// handle, not a snapshot lease for every read. Callers saving a loaded
    /// config snapshot use [`Self::write_versioned`] with that snapshot's
    /// captured revision instead.
    async fn write(&self, toml: &str) -> Result<()>;

    /// Persist a config snapshot against the revision it was loaded from.
    /// Document sources use `None` for a create-only write; file sources have
    /// no revisions and use this default implementation.
    async fn write_versioned(
        &self,
        toml: &str,
        _expected: Option<tinystoragedrivers::Version>,
    ) -> Result<Option<tinystoragedrivers::Version>> {
        self.write(toml).await?;
        Ok(None)
    }
}

/// The source for the config at `config_path`.
///
/// The document source when the host installed a shared (multi-tenant)
/// backend and the current scope resolves; the file otherwise, which includes
/// every desktop build and SaaS calls with no resolvable scope. The scope is
/// the ambient one, as for every other storage-backed domain.
///
/// # Errors
///
/// On a shared backend with a scope, when no master key can derive the
/// scope's data key: the config then fails closed instead of falling back to
/// a file that would hold the same settings unprotected.
pub(crate) fn for_config(config_path: &Path) -> Result<Box<dyn ConfigSource>> {
    let file = FileConfigSource::new(config_path);
    #[cfg(test)]
    if let Some(forced) = tests_support::forced_document_scope() {
        return Ok(Box::new(DocumentConfigSource::new(
            std::sync::Arc::clone(forced.scoped.documents()),
            forced.scope,
            forced.keys,
            file,
            DocumentBackendIdentity::Test(forced.identity),
        )));
    }
    if !crate::storage::installed_is_shared() {
        return Ok(Box::new(file));
    }
    let resolved = crate::storage::current_scope().map(|scope| {
        crate::storage::installed().map(|backend| {
            let scoped = backend.for_scope(&scope);
            scoped.map(|scoped| (scoped, scope, backend))
        })
    });
    match resolved {
        Ok(Some(Ok((scoped, scope, backend)))) => {
            let keys = crate::storage::secrets::keys()
                .map_err(|error| anyhow::anyhow!("config document key: {error}"))?;
            Ok(Box::new(DocumentConfigSource::new(
                std::sync::Arc::clone(scoped.documents()),
                scope,
                keys,
                file,
                DocumentBackendIdentity::Installed(backend),
            )))
        }
        Ok(None) => Ok(Box::new(file)),
        Ok(Some(Err(error))) => Err(error.into()),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "[config] no storage scope for the config document; using the file"
            );
            Ok(Box::new(file))
        }
    }
}

/// `toml` without its [`BOOTSTRAP_TABLES`].
pub(crate) fn strip_bootstrap(toml_text: &str) -> Result<String> {
    let mut table: toml::Table = toml::from_str(toml_text)?;
    for name in BOOTSTRAP_TABLES {
        table.remove(*name);
    }
    Ok(toml::to_string_pretty(&table)?)
}

/// `stored` with its bootstrap tables replaced by those of `bootstrap_text`
/// (the file's), or removed when the file has none.
pub(crate) fn apply_bootstrap(stored: &str, bootstrap_text: Option<&str>) -> Result<String> {
    let mut table: toml::Table = toml::from_str(stored)?;
    // A bootstrap file that does not parse is an error, not an empty file:
    // treating it as empty would silently drop the storage table.
    let bootstrap: toml::Table = match bootstrap_text {
        Some(text) => {
            toml::from_str(text).context("the bootstrap config file is not valid TOML")?
        }
        None => toml::Table::default(),
    };
    for name in BOOTSTRAP_TABLES {
        table.remove(*name);
        if let Some(value) = bootstrap.get(*name) {
            table.insert((*name).to_string(), value.clone());
        }
    }
    Ok(toml::to_string_pretty(&table)?)
}

#[cfg(test)]
pub(crate) mod tests_support;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
