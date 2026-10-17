//! [`DocumentConfigSource`]: the config as the `config/{scope}` document.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::json;
use tinystoragedrivers::secrets::{crypto, KeyProvider};
use tinystoragedrivers::{CollectionSpec, DocumentStore, Precondition};

use super::{apply_bootstrap, strip_bootstrap, ConfigRead, ConfigSource, FileConfigSource};
use crate::storage::Scope;

use super::storage_schema::document_id;
pub(crate) use super::storage_schema::CONFIG_COLLECTION;

/// The document field holding the encrypted TOML body (`enc2:` hex).
const BODY_FIELD: &str = "toml_enc";

/// A config kept as one document, `config/{scope}`, on the storage backend.
///
/// The body is the TOML text without its bootstrap tables
/// ([`super::BOOTSTRAP_TABLES`]), encrypted as a whole (`enc2:`) under the
/// scope's data key from the storage [`KeyProvider`]: the same per-scope key
/// the scope's other secrets use, derived from a master key every node of a
/// deployment shares. Field-level encryption under the process-local config
/// key would leave the document unreadable on any other node, so
/// [`ConfigSource::encrypts_body`] tells `Config::save` to skip it. With no
/// key the source fails closed.
///
/// Reads re-apply the bootstrap tables from the `file` fallback. When the
/// scope has no document yet, reads fall back to the file (a first run seeds
/// from it) and the next write creates the document.
pub(crate) struct DocumentConfigSource {
    docs: Arc<dyn DocumentStore>,
    scope: Scope,
    keys: Arc<dyn KeyProvider>,
    file: FileConfigSource,
}

impl DocumentConfigSource {
    pub(crate) fn new(
        docs: Arc<dyn DocumentStore>,
        scope: Scope,
        keys: Arc<dyn KeyProvider>,
        file: FileConfigSource,
    ) -> Self {
        Self {
            docs,
            scope,
            keys,
            file,
        }
    }

    fn key(&self) -> Result<zeroize::Zeroizing<[u8; crypto::KEY_LEN]>> {
        self.keys
            .data_key(&self.scope)
            .map_err(|error| anyhow!("no data key for the config document: {error}"))
    }

    /// The local file's text for its bootstrap tables.
    ///
    /// When the file read had to recover (non-UTF-8 content renamed aside, the
    /// `.bak` used), the recovered text is written back to the file: saves go
    /// to the document, so nothing else would ever recreate it, and a restart
    /// whose storage URL lives only in the file's `[storage]` table would boot
    /// without its shared backend. Nothing recoverable means no bootstrap
    /// tables, the same as an absent file (the corrupt bytes stay preserved
    /// beside it).
    async fn read_bootstrap_file(&self) -> Result<String> {
        let read = self.file.read().await?;
        if read.recovered {
            if read.contents.is_empty() {
                tracing::error!(
                    path = %self.file.path().display(),
                    "[config] bootstrap config file unreadable and no backup; \
                     reading the config document without bootstrap tables"
                );
            } else if let Err(error) = self.file.write(&read.contents).await {
                tracing::warn!(
                    path = %self.file.path().display(),
                    error = %error,
                    "[config] could not restore the recovered bootstrap config file"
                );
            } else {
                tracing::warn!(
                    path = %self.file.path().display(),
                    "[config] bootstrap config file was corrupt; restored it from its backup"
                );
            }
        }
        Ok(read.contents)
    }

    /// The decrypted stored body, or `None` when the scope has no document.
    async fn stored_body(&self) -> Result<Option<zeroize::Zeroizing<String>>> {
        self.docs
            .ensure_collection(&CollectionSpec::new(CONFIG_COLLECTION))
            .await
            .context("declare the config collection")?;
        let stored = self
            .docs
            .get(CONFIG_COLLECTION, &document_id(self.scope.as_str())?)
            .await
            .context("read the config document")?;
        let Some(versioned) = stored else {
            return Ok(None);
        };
        // A document that exists but has no sealed body is corruption, not
        // absence: reading the file instead would let the next save overwrite
        // the tenant's record with stale data.
        let sealed = versioned
            .doc
            .get(BODY_FIELD)
            .and_then(|body| body.as_str())
            .ok_or_else(|| anyhow!("the config document has no sealed `{BODY_FIELD}` body"))?
            .to_string();
        let key = self.key()?;
        let plain = crypto::decrypt_enc2(&key, &sealed)
            .map_err(|error| anyhow!("the config document does not decrypt: {error}"))?;
        let body = String::from_utf8(plain.to_vec())
            .map_err(|_| anyhow!("the config document is not UTF-8"))?;
        Ok(Some(zeroize::Zeroizing::new(body)))
    }
}

#[async_trait]
impl ConfigSource for DocumentConfigSource {
    fn label(&self) -> &'static str {
        "document"
    }

    fn encrypts_body(&self) -> bool {
        true
    }

    async fn exists(&self) -> Result<bool> {
        // Existence is checked without decrypting: a document whose key is
        // missing still exists, and the read then reports the real error.
        self.docs
            .ensure_collection(&CollectionSpec::new(CONFIG_COLLECTION))
            .await
            .context("declare the config collection")?;
        let stored = self
            .docs
            .get(CONFIG_COLLECTION, &document_id(self.scope.as_str())?)
            .await
            .context("read the config document")?;
        if stored.is_some() {
            return Ok(true);
        }
        self.file.exists().await
    }

    async fn read(&self) -> Result<ConfigRead> {
        let Some(body) = self.stored_body().await? else {
            tracing::debug!(scope = %self.scope.as_str(), "[config] no config document; reading the file");
            return self.file.read().await;
        };
        // Bootstrap tables come from the file, whatever the document says.
        // Through the file source, so its corruption recovery (a `.bak`, a
        // non-UTF-8 file renamed aside) applies to the bootstrap read too.
        let file_text = if self.file.exists().await? {
            Some(self.read_bootstrap_file().await?)
        } else {
            None
        };
        let contents = apply_bootstrap(&body, file_text.as_deref())
            .context("apply the bootstrap tables to the config document")?;
        // The document body itself was read intact. A recovered bootstrap file
        // is handled (and restored) by `read_bootstrap_file`, not reported
        // here: the loader's file-corruption branch would rename the local
        // `config.toml` aside and save only to the document, losing the
        // bootstrap tables on the next boot.
        Ok(ConfigRead {
            contents,
            recovered: false,
            from_document: true,
        })
    }

    async fn write(&self, toml: &str) -> Result<()> {
        let body = zeroize::Zeroizing::new(
            strip_bootstrap(toml).context("strip bootstrap tables from the config")?,
        );
        let key = self.key()?;
        let sealed = crypto::encrypt_enc2(&key, body.as_bytes())
            .map_err(|error| anyhow!("encrypt the config document: {error}"))?;
        self.docs
            .ensure_collection(&CollectionSpec::new(CONFIG_COLLECTION))
            .await
            .context("declare the config collection")?;
        self.docs
            .put(
                CONFIG_COLLECTION,
                &document_id(self.scope.as_str())?,
                json!({ BODY_FIELD: sealed }),
                Precondition::None,
            )
            .await
            .context("write the config document")?;
        tracing::debug!(scope = %self.scope.as_str(), "[config] wrote the config document");
        Ok(())
    }
}
