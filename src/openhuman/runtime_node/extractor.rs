//! Archive extraction for downloaded Node.js distributions.
//!
//! Handles both shapes that nodejs.org ships:
//!
//! * `.tar.xz` on macOS and Linux — decoded via `xz2` then unpacked through
//!   the `tar` crate.
//! * `.zip` on Windows — unpacked through the `zip` crate.
//!
//! All archives are "single-rooted": they expand into one top-level folder
//! like `node-v22.11.0-darwin-arm64/`. We extract into a caller-supplied
//! staging directory, then return the absolute path of that inner folder so
//! the bootstrap layer can rename/move it into the cache atomically.
//!
//! Extraction is CPU/IO-bound and the underlying crates are synchronous, so
//! we wrap the real work in `tokio::task::spawn_blocking` to keep the
//! runtime responsive.

use anyhow::{anyhow, Context, Result};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

/// Extract `archive` into `extract_root` and return the absolute path of the
/// single top-level folder produced by the archive.
///
/// `is_zip = true` selects the zip path, otherwise the tar.xz path runs.
/// On any error the caller should treat `extract_root` as contaminated and
/// remove it before retrying — we do not auto-clean because the caller
/// typically owns a fresh temp dir.
pub async fn extract_distribution(
    archive: &Path,
    extract_root: &Path,
    is_zip: bool,
) -> Result<PathBuf> {
    let archive = archive.to_path_buf();
    let extract_root = extract_root.to_path_buf();

    tracing::info!(
        archive = %archive.display(),
        extract_root = %extract_root.display(),
        is_zip,
        "[node_runtime::extractor] starting extraction"
    );

    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        fs::create_dir_all(&extract_root)
            .with_context(|| format!("creating extract root {}", extract_root.display()))?;

        if is_zip {
            extract_zip(&archive, &extract_root)?;
        } else {
            extract_tar_xz(&archive, &extract_root)?;
        }

        let top_level = find_single_top_level(&extract_root)?;
        tracing::info!(
            top_level = %top_level.display(),
            "[node_runtime::extractor] extraction complete"
        );
        Ok(top_level)
    })
    .await
    .context("spawn_blocking join failure during extraction")?
}

/// Extract a `.tar.xz` archive into `extract_root`.
fn extract_tar_xz(archive: &Path, extract_root: &Path) -> Result<()> {
    let file =
        File::open(archive).with_context(|| format!("opening archive {}", archive.display()))?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    // `set_preserve_permissions(true)` is the default on Unix; we restate
    // it so the `node` binary keeps its `+x` bit after extraction.
    tar.set_preserve_permissions(true);
    tar.set_overwrite(true);
    tar.unpack(extract_root)
        .with_context(|| format!("unpacking tar.xz into {}", extract_root.display()))?;
    Ok(())
}

/// Extract a `.zip` archive into `extract_root`. Handles directory entries,
/// file entries, and restores Unix mode bits where present (no-op on
/// Windows hosts, which is where `.zip` actually matters).
fn extract_zip(archive: &Path, extract_root: &Path) -> Result<()> {
    let file =
        File::open(archive).with_context(|| format!("opening archive {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("opening zip archive {}", archive.display()))?;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .with_context(|| format!("reading zip entry {i}"))?;
        let Some(relative) = entry.enclosed_name() else {
            tracing::warn!(
                name = entry.name(),
                "[node_runtime::extractor] skipping zip entry with unsafe path"
            );
            continue;
        };
        let out_path = extract_root.join(relative);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .with_context(|| format!("creating {}", out_path.display()))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            let mut out = File::create(&out_path)
                .with_context(|| format!("creating {}", out_path.display()))?;
            io::copy(&mut entry, &mut out)
                .with_context(|| format!("writing {}", out_path.display()))?;
        }

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))
                .with_context(|| format!("chmod {}", out_path.display()))?;
        }
    }

    Ok(())
}

/// Locate the single top-level directory inside `extract_root`. Node.js
/// archives always produce one root folder; anything else (multiple
/// entries, only files) is a contract violation from our side and we
/// surface it as an error rather than guessing.
fn find_single_top_level(extract_root: &Path) -> Result<PathBuf> {
    let mut entries = fs::read_dir(extract_root)
        .with_context(|| format!("listing {}", extract_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("reading entries of {}", extract_root.display()))?;

    // Stable order for deterministic logging.
    entries.sort_by_key(|e| e.file_name());

    let mut dirs: Vec<PathBuf> = entries
        .into_iter()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();

    match dirs.len() {
        1 => Ok(dirs.pop().unwrap()),
        0 => Err(anyhow!(
            "expected one top-level folder under {}, found none",
            extract_root.display()
        )),
        n => Err(anyhow!(
            "expected one top-level folder under {}, found {n}: {:?}",
            extract_root.display(),
            dirs
        )),
    }
}

/// Atomically move `staged` into place at `final_dest`.
///
/// Strategy:
/// 1. If `final_dest` already exists, move it to a sibling `.old-<pid>`
///    path so we never lose a working install even if a later step fails.
/// 2. Rename `staged` -> `final_dest`. On the same filesystem this is a
///    single `rename(2)` and is atomic from the reader's perspective.
/// 3. Best-effort cleanup of the `.old-*` directory.
///
/// Returns the `final_dest` path on success.
pub async fn atomic_install(staged: &Path, final_dest: &Path) -> Result<PathBuf> {
    let staged = staged.to_path_buf();
    let final_dest = final_dest.to_path_buf();

    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        if let Some(parent) = final_dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating parent {}", parent.display()))?;
        }

        let mut backup: Option<PathBuf> = None;
        if final_dest.exists() {
            let ts = std::process::id();
            let candidate = final_dest.with_extension(format!("old-{ts}"));
            fs::rename(&final_dest, &candidate).with_context(|| {
                format!(
                    "moving existing install {} aside to {}",
                    final_dest.display(),
                    candidate.display()
                )
            })?;
            backup = Some(candidate);
        }

        if let Err(err) = fs::rename(&staged, &final_dest).with_context(|| {
            format!(
                "renaming staged {} -> {}",
                staged.display(),
                final_dest.display()
            )
        }) {
            // Stage->final rename failed; restore the previous install from
            // backup so the working runtime stays in place. Surface any
            // restore failure separately (as a warning) but always return
            // the original error.
            if let Some(backup_path) = backup.as_ref() {
                if let Err(restore_err) = fs::rename(backup_path, &final_dest) {
                    tracing::warn!(
                        backup = %backup_path.display(),
                        final_dest = %final_dest.display(),
                        error = %restore_err,
                        "[node_runtime::extractor] failed to restore backup after staged rename failure"
                    );
                } else {
                    tracing::info!(
                        final_dest = %final_dest.display(),
                        "[node_runtime::extractor] restored previous install after staged rename failure"
                    );
                }
            }
            return Err(err);
        }

        if let Some(path) = backup {
            let _ = fs::remove_dir_all(&path);
        }
        tracing::info!(
            final_dest = %final_dest.display(),
            "[node_runtime::extractor] atomic install complete"
        );
        Ok(final_dest)
    })
    .await
    .context("spawn_blocking join failure during atomic install")?
}
