//! One-time, read-only import of tinyjuice's on-disk CCR tier into the
//! module's blob storage.
//!
//! The compress-cache-retrieve tier used to write each offloaded original to
//! `<workspace>/.tokenjuice/ccr/<token>`, expiring by file age. From v0.10.0
//! the module keeps them as expiring blobs `tinyjuice_ccr/<token>` in its own
//! scope and writes no files. A model that was handed a recovery handle before
//! the upgrade can still fetch the original afterwards.
//!
//! * **Legacy files are never modified or deleted.** They are only read.
//! * **Nothing is overwritten.** A blob that already exists is kept.
//! * **Lifetime is kept.** A file older than the configured TTL is dropped, as
//!   the old tier would have on its next read; a younger one is written with
//!   the time it has left. With no TTL configured originals are kept.
//! * **Runs once per directory.** A marker document (in a collection the
//!   module's grant does not include) records it. An unreadable file writes no
//!   marker, so the next start retries. A name that is not a CCR token, or a
//!   file that is not UTF-8 text, is skipped.
//! * **No expiry support, no import.** A backend whose blob store cannot
//!   expire blobs leaves the module on its in-memory cache (it refuses a TTL
//!   it cannot honour), so nothing is written for it to find.

use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use tinystoragedrivers::{Capability, ScopedStorage};

use super::storage_grant_schemas::tinyjuice::{CCR_BLOB_PREFIX, MODULE_ID};
use super::storage_import_support as support;
use crate::core::runtime::DispatchCtx;

const LOG_PREFIX: &str = "[tinyjuice-import]";
/// `tinyjuice::cache::store::short_hash`: 16 bytes, hex encoded.
const TOKEN_LEN: usize = 32;
const CONTENT_TYPE: &str = "text/plain; charset=utf-8";
/// Host-only: not in the module's grant, so the module cannot see it.
const MARKER_COLLECTION: &str = "tinyjuice_host_import";

/// How one import attempt ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// No storage backend is installed; the module is not using this storage.
    NoBackend,
    /// The marker says this directory was already imported.
    AlreadyDone,
    /// The directory does not exist.
    NoSource,
    /// The backend cannot expire blobs, so the module does not use it.
    Unsupported,
    /// The files were read and their originals written.
    Imported {
        imported: usize,
        expired: usize,
        skipped: usize,
    },
}

/// Imports the CCR files under `root` into the module's scope of the installed
/// backend. `ttl` is the configured original lifetime; `None` keeps them.
pub(crate) async fn import_legacy(
    ctx: &DispatchCtx,
    root: &Path,
    ttl: Option<Duration>,
) -> Result<Outcome> {
    let Some(scoped) = support::module_scope(ctx, MODULE_ID)? else {
        return Ok(Outcome::NoBackend);
    };
    import_into(ctx, &scoped, root, ttl, SystemTime::now()).await
}

/// One readable original: its token, text and age.
struct LegacyOriginal {
    token: String,
    text: String,
    age: Duration,
}

async fn import_into(
    ctx: &DispatchCtx,
    scoped: &ScopedStorage,
    root: &Path,
    ttl: Option<Duration>,
    now: SystemTime,
) -> Result<Outcome> {
    let marker = support::marker_id("ccr", root);
    if support::already_imported(scoped, MARKER_COLLECTION, &marker).await? {
        return Ok(Outcome::AlreadyDone);
    }
    if !root.is_dir() {
        return Ok(Outcome::NoSource);
    }
    let blobs = scoped.blobs();
    if ttl.is_some() && !blobs.capabilities().contains(Capability::BlobTtl) {
        return Ok(Outcome::Unsupported);
    }

    let dir = root.to_owned();
    let (originals, mut skipped) = ctx
        .spawn_blocking(move || read_originals(&dir, now))
        .await
        .context("the legacy reader panicked")??;

    let (mut imported, mut expired) = (0usize, 0usize);
    for original in originals {
        let remaining = match ttl {
            Some(ttl) => match ttl.checked_sub(original.age).filter(|left| !left.is_zero()) {
                Some(left) => Some(left),
                None => {
                    expired += 1;
                    continue;
                }
            },
            None => None,
        };
        let key = format!("{CCR_BLOB_PREFIX}{}", original.token);
        let present = blobs
            .head(&key)
            .await
            .map_err(|error| anyhow!("check a blob: {:?}", error.kind()))?
            .is_some();
        if present {
            skipped += 1;
            continue;
        }
        let bytes = original.text.into_bytes();
        match remaining {
            Some(left) => {
                blobs
                    .put_expiring(&key, bytes, Some(CONTENT_TYPE), left)
                    .await
            }
            None => blobs.put(&key, bytes, Some(CONTENT_TYPE)).await,
        }
        .map_err(|error| anyhow!("write a blob: {:?}", error.kind()))?;
        imported += 1;
    }

    support::mark_imported(scoped, MARKER_COLLECTION, &marker).await?;
    tracing::info!("{LOG_PREFIX} imported={imported} expired={expired} skipped={skipped}");
    Ok(Outcome::Imported {
        imported,
        expired,
        skipped,
    })
}

fn is_token(name: &str) -> bool {
    name.len() == TOKEN_LEN && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The modified time and bytes of a regular file, or `None` for a link,
/// directory or anything else; the file is never reached through a symlink.
fn read_regular(path: &Path) -> Result<Option<(SystemTime, Vec<u8>)>> {
    use std::io::Read;
    let mut options = std::fs::File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK keeps a FIFO planted in the directory from hanging the open.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        // O_NOFOLLOW refuses a symlink with ELOOP.
        Err(error) if is_link_refusal(&error) => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("open {}", path.display())),
    };
    let meta = file
        .metadata()
        .with_context(|| format!("stat {}", path.display()))?;
    if !meta.file_type().is_file() {
        return Ok(None);
    }
    let modified = meta
        .modified()
        .with_context(|| format!("stat {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(Some((modified, bytes)))
}

#[cfg(unix)]
fn is_link_refusal(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(not(unix))]
fn is_link_refusal(_: &std::io::Error) -> bool {
    false
}

fn read_originals(root: &Path, now: SystemTime) -> Result<(Vec<LegacyOriginal>, usize)> {
    let mut originals = Vec::new();
    let mut skipped = 0usize;
    let mut entries = std::fs::read_dir(root)
        .with_context(|| format!("list {}", root.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("list {}", root.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let Some(token) = name.to_str().filter(|name| is_token(name)) else {
            skipped += 1;
            continue;
        };
        let path = entry.path();
        // Open without following a link (it could point outside the CCR
        // directory) and read both metadata and bytes from that one handle.
        let Some((modified, bytes)) = read_regular(&path)? else {
            skipped += 1;
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            skipped += 1;
            continue;
        };
        originals.push(LegacyOriginal {
            // The module keys blobs by the lowercase token it minted.
            token: token.to_ascii_lowercase(),
            text,
            age: now.duration_since(modified).unwrap_or_default(),
        });
    }
    Ok((originals, skipped))
}

#[cfg(test)]
#[path = "storage_import_tinyjuice_tests.rs"]
mod tests;
