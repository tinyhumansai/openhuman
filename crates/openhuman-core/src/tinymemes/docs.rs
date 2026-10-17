//! The learned-state documents the module asks the host to keep: the slang
//! index, the meme index and the ring of remixed message ids.
//!
//! They live on the storage document port ([`crate::storage`]) under the
//! acting scope, in `memes.state`, one document per [`IndexKind`]. The host
//! never interprets them: the formats, the ring and its cap belong to the
//! module, and the text-to-body mapping to `tinymemes_bus::storage`
//! (`state_body` / `state_text`), which also declares the step that brought
//! the v1 `tinymemes_documents` rows (`{"json": "<text>"}`) here.
//!
//! Older installs kept them as `<workspace>/tinymemes/{slang-index,
//! meme-index,remixed}.json`. The first load of a kind with no stored document
//! adopts that file's content once and renames the file to `<name>.migrated`,
//! so nothing is lost and the file is never read again.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use tinymemes_bus::storage::{state_body, state_id, state_text, STATE_COLLECTION as COLLECTION};
use tinymemes_bus::IndexKind;
use tinystoragedrivers::{CollectionSpec, Precondition};

use crate::config::Config;
use crate::storage::documents::Repo;

const DOMAIN: &str = "tinymemes::docs";
/// Directory of the pre-storage files, under the workspace.
const LEGACY_DIR: &str = "tinymemes";

fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(COLLECTION)]
}

fn doc_id(kind: IndexKind) -> &'static str {
    state_id(kind)
}

fn legacy_file(kind: IndexKind) -> &'static str {
    match kind {
        IndexKind::Slang => "slang-index.json",
        IndexKind::Memes => "meme-index.json",
        IndexKind::Remixed => "remixed.json",
    }
}

fn legacy_path(workspace_dir: &Path, kind: IndexKind) -> PathBuf {
    workspace_dir.join(LEGACY_DIR).join(legacy_file(kind))
}

fn repo(config: &Config) -> Result<Repo> {
    Repo::current_for(&config.workspace_dir, DOMAIN, collections)
}

/// The stored document for `kind`, adopting the legacy file once.
pub(super) fn load(config: &Config, kind: IndexKind) -> Result<Option<String>> {
    let repo = repo(config)?;
    let id = doc_id(kind);
    let stored = repo.run(move |docs| async move { docs.get(COLLECTION, id).await })?;
    if let Some(stored) = stored {
        return Ok(Some(state_text(kind, &stored.doc)));
    }
    let path = legacy_path(&config.workspace_dir, kind);
    // Only an absent file means "nothing to adopt". Any other failure
    // (permissions, bad UTF-8) is an error: answering `None` would let the
    // module start empty and save over state the old file still holds.
    let legacy = match std::fs::read_to_string(&path) {
        Ok(legacy) => legacy,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow!("read {}: {e}", path.display())),
    };
    // `Absent`: a concurrent save wins over the old file's content.
    let adopted = state_body(kind, &legacy);
    let written = repo.run(move |docs| async move {
        docs.put(COLLECTION, id, adopted, Precondition::Absent)
            .await
            .map(|_| ())
    });
    match written {
        Ok(()) => {
            let migrated = path.with_extension("json.migrated");
            match std::fs::rename(&path, &migrated) {
                Ok(()) => log::info!(
                    "[tinymemes] adopted {} into storage (kept as {})",
                    legacy_file(kind),
                    migrated.display()
                ),
                Err(e) => log::warn!("[tinymemes] cannot retire {}: {e}", path.display()),
            }
            Ok(Some(legacy))
        }
        // Somebody stored a document in between; theirs is the truth. When no
        // document exists the put failed for a real reason, which is reported.
        Err(error) => {
            let stored = repo.run(move |docs| async move { docs.get(COLLECTION, id).await })?;
            match stored {
                Some(stored) => Ok(Some(state_text(kind, &stored.doc))),
                None => Err(error),
            }
        }
    }
}

/// Replace the stored document for `kind`.
pub(super) fn save(config: &Config, kind: IndexKind, json: String) -> Result<()> {
    let repo = repo(config)?;
    let id = doc_id(kind);
    repo.run(move |docs| async move {
        docs.put(COLLECTION, id, state_body(kind, &json), Precondition::None)
            .await
            .map(|_| ())
    })
    .map_err(|e| anyhow!("save {id}: {e}"))
}

#[cfg(test)]
#[path = "docs_tests.rs"]
mod tests;
