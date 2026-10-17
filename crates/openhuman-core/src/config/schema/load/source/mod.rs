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
}

/// A place a config's TOML text is read from and written to.
#[async_trait]
pub(crate) trait ConfigSource: Send + Sync {
    /// A short, stable name for logs: `file` or `document`.
    fn label(&self) -> &'static str;

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

    /// Replace the config text. Returns `Err` only while the previous config is
    /// still the live one, so callers may roll back in-memory state on `Err`.
    async fn write(&self, toml: &str) -> Result<()>;
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
        )));
    }
    if !crate::storage::installed_is_shared() {
        return Ok(Box::new(file));
    }
    let resolved = crate::storage::current_scoped().and_then(|scoped| {
        let scope = crate::storage::current_scope()?;
        Ok(scoped.map(|scoped| (scoped, scope)))
    });
    match resolved {
        Ok(Some((scoped, scope))) => {
            let keys = crate::storage::secrets::keys()
                .map_err(|error| anyhow::anyhow!("config document key: {error}"))?;
            Ok(Box::new(DocumentConfigSource::new(
                std::sync::Arc::clone(scoped.documents()),
                scope,
                keys,
                file,
            )))
        }
        Ok(None) => Ok(Box::new(file)),
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
