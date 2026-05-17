//! Archive extraction for managed standalone Python distributions.

use anyhow::{anyhow, Context, Result};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

pub async fn extract_distribution(archive: &Path, extract_root: &Path) -> Result<PathBuf> {
    let archive = archive.to_path_buf();
    let extract_root = extract_root.to_path_buf();

    tracing::info!(
        archive = %archive.display(),
        extract_root = %extract_root.display(),
        "[runtime_python::extractor] starting extraction"
    );

    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        fs::create_dir_all(&extract_root)
            .with_context(|| format!("creating extract root {}", extract_root.display()))?;

        let file = File::open(&archive)
            .with_context(|| format!("opening archive {}", archive.display()))?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(decoder);
        tar.set_preserve_permissions(true);
        tar.set_overwrite(true);
        tar.unpack(&extract_root)
            .with_context(|| format!("unpacking tar.gz into {}", extract_root.display()))?;

        find_single_top_level(&extract_root)
    })
    .await
    .context("spawn_blocking join failure during extraction")?
}

fn find_single_top_level(extract_root: &Path) -> Result<PathBuf> {
    let mut entries = fs::read_dir(extract_root)
        .with_context(|| format!("listing {}", extract_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("reading entries of {}", extract_root.display()))?;
    entries.sort_by_key(|e| e.file_name());

    let mut dirs = entries
        .into_iter()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect::<Vec<_>>();

    match dirs.len() {
        1 => Ok(dirs.pop().expect("single dir")),
        0 => Err(anyhow!(
            "expected one top-level folder under {}, found none",
            extract_root.display()
        )),
        n => Err(anyhow!(
            "expected one top-level folder under {}, found {n}",
            extract_root.display()
        )),
    }
}

pub async fn atomic_install(staged: &Path, final_dest: &Path) -> Result<PathBuf> {
    let staged = staged.to_path_buf();
    let final_dest = final_dest.to_path_buf();

    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        if let Some(parent) = final_dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating parent {}", parent.display()))?;
        }

        let backup = if final_dest.exists() {
            let candidate = final_dest.with_extension(format!("old-{}", std::process::id()));
            fs::rename(&final_dest, &candidate).with_context(|| {
                format!(
                    "moving existing install {} aside to {}",
                    final_dest.display(),
                    candidate.display()
                )
            })?;
            Some(candidate)
        } else {
            None
        };

        if let Err(err) = fs::rename(&staged, &final_dest).with_context(|| {
            format!(
                "renaming staged {} -> {}",
                staged.display(),
                final_dest.display()
            )
        }) {
            if let Some(backup_path) = backup.as_ref() {
                if let Err(restore_err) = fs::rename(backup_path, &final_dest) {
                    return Err(anyhow!(
                        "{err}; rollback from {} to {} also failed: {restore_err}",
                        backup_path.display(),
                        final_dest.display()
                    ));
                }
            }
            return Err(err);
        }

        if let Some(backup_path) = backup {
            let _ = fs::remove_dir_all(backup_path);
        }

        Ok(final_dest)
    })
    .await
    .context("spawn_blocking join failure during atomic install")?
}
