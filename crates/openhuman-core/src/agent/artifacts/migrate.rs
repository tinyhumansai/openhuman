//! Move legacy artifact files into the visible files folder (#5505).
//!
//! Artifacts created before the files folder existed keep their bytes under
//! the hidden `<workspace>/artifacts/<id>/`. At boot each account's legacy
//! `Ready` records are moved to the files folder so the user can find them.
//! Records (and a legacy `meta.json` / `args.json`) never move.
//!
//! Each file goes through four steps, ordered so that at every point the
//! record still resolves to a complete file and nothing is lost:
//!
//! 1. copy the bytes to a hidden `.<id>.partial` in the files folder and
//!    flush them to disk (a copy, not a rename: the two folders can sit on
//!    different volumes);
//! 2. claim a human-readable name (`create_new`, suffixed on collision) and
//!    rename the partial onto it;
//! 3. rewrite the record atomically to point at the new file;
//! 4. only then delete the legacy bytes.
//!
//! A crash before step 3 leaves the record on its legacy path, and the next
//! boot migrates again; the worst leftover is one duplicate visible copy
//! (after step 2) or a `.partial` (after step 1), which the next run removes.
//! A crash after step 3 leaves a legacy copy the next run deletes. Running it
//! again on a migrated workspace changes nothing.

use std::path::{Path, PathBuf};

use super::files;
use super::store::{artifacts_root, list_artifacts, save_artifact_meta};
use super::types::{ArtifactMeta, ArtifactStatus};

/// What one migration pass did, for the boot log and tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Records whose file was moved into the files folder.
    pub moved: usize,
    /// Legacy copies removed for records already pointing at the files folder.
    pub cleaned: usize,
    /// Records left on their legacy path because a step failed.
    pub failed: usize,
}

/// A step to stop after, simulating a crash. Test-only in effect: the public
/// entry point always passes `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrashAfter {
    Copy,
    Rename,
    MetaWrite,
}

const PARTIAL_SUFFIX: &str = ".partial";
/// Files in `<workspace>/artifacts/<id>/` that are metadata, never bytes.
const METADATA_FILES: &[&str] = &["meta.json", "args.json", "meta.json.tmp"];

/// Migrate every legacy `Ready` artifact in `workspace_dir` into `files_dir`.
/// Idempotent and safe to run on every boot.
// keep until v0.66 (review after 2027-01-11): offline workspaces can still hold
// pre-files Ready artifacts; remove only with an explicit upgrade/import boundary.
pub async fn migrate_legacy_artifacts(workspace_dir: &Path, files_dir: &Path) -> MigrationReport {
    migrate_with(workspace_dir, files_dir, None).await
}

pub(crate) async fn migrate_with(
    workspace_dir: &Path,
    files_dir: &Path,
    crash_after: Option<CrashAfter>,
) -> MigrationReport {
    let mut report = MigrationReport::default();
    let Ok(root) = artifacts_root(workspace_dir).await else {
        return report;
    };
    // The records live in the storage backend (a legacy `meta.json` is
    // imported on this first read); only the bytes are still files.
    let all = match list_artifacts(workspace_dir, 0, usize::MAX, None).await {
        Ok((all, _)) => all,
        Err(e) => {
            log::warn!("[artifacts][migrate] cannot list records: {e}");
            return report;
        }
    };
    let mut records = Vec::new();
    for meta in all {
        if meta.id.is_empty() || meta.id == "." || meta.id == ".." || meta.id.contains(['/', '\\'])
        {
            continue;
        }
        records.push((root.join(&meta.id), meta));
    }
    if records.is_empty() {
        return report;
    }
    remove_stale_partials(files_dir).await;

    for (artifact_dir, meta) in records {
        if meta.file.is_some() {
            // Already migrated; a crash after step 3 may have left the legacy
            // copy behind.
            report.cleaned += remove_legacy_leftovers(&artifact_dir).await;
            continue;
        }
        if !matches!(meta.status, ArtifactStatus::Ready) {
            continue;
        }
        let Some(src) = legacy_bytes(&root, &artifact_dir, &meta) else {
            continue;
        };
        match migrate_one(workspace_dir, files_dir, &meta, &src, crash_after).await {
            Ok(true) => report.moved += 1,
            Ok(false) => {}
            Err(e) => {
                report.failed += 1;
                log::warn!(
                    "[artifacts][migrate] id={} left on legacy path: {e}",
                    meta.id
                );
            }
        }
        if crash_after.is_some() {
            break;
        }
    }
    log::info!(
        "[artifacts][migrate] moved={} cleaned={} failed={}",
        report.moved,
        report.cleaned,
        report.failed
    );
    report
}

/// The legacy on-disk bytes of an unmigrated record, when present and inside
/// its own artifact directory.
fn legacy_bytes(root: &Path, artifact_dir: &Path, meta: &ArtifactMeta) -> Option<PathBuf> {
    let rel = Path::new(&meta.path);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let candidate = root.join(rel);
    (candidate.starts_with(artifact_dir) && candidate.is_file()).then_some(candidate)
}

/// For a migrated record, every regular file in its artifact directory other
/// than the metadata sidecars is a legacy copy left by a crash between steps 3
/// and 4. Returns how many were removed.
async fn remove_legacy_leftovers(artifact_dir: &Path) -> usize {
    let Ok(mut entries) = tokio::fs::read_dir(artifact_dir).await else {
        return 0;
    };
    let mut removed = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if METADATA_FILES.contains(&name.as_str()) {
            continue;
        }
        if matches!(entry.file_type().await, Ok(ft) if ft.is_file())
            && tokio::fs::remove_file(entry.path()).await.is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Returns `Ok(true)` when the file was moved, `Ok(false)` on a simulated crash.
async fn migrate_one(
    workspace_dir: &Path,
    files_dir: &Path,
    meta: &ArtifactMeta,
    src: &Path,
    crash_after: Option<CrashAfter>,
) -> Result<bool, String> {
    tokio::fs::create_dir_all(files_dir)
        .await
        .map_err(|e| format!("create files folder: {e}"))?;

    // Step 1: copy to a hidden partial and flush it.
    let partial = files_dir.join(format!(".{}{PARTIAL_SUFFIX}", meta.id));
    let _ = tokio::fs::remove_file(&partial).await;
    tokio::fs::copy(src, &partial)
        .await
        .map_err(|e| format!("copy: {e}"))?;
    tokio::fs::File::open(&partial)
        .await
        .map_err(|e| format!("reopen partial: {e}"))?
        .sync_all()
        .await
        .map_err(|e| format!("flush partial: {e}"))?;
    if crash_after == Some(CrashAfter::Copy) {
        return Ok(false);
    }

    // Step 2: claim a readable name and move the partial onto it.
    let file_name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (file_name.clone(), "bin".to_string()),
    };
    let target = files::reserve_file(files_dir, &stem, &ext, &meta.id).await?;
    if let Err(e) = tokio::fs::rename(&partial, &target).await {
        let _ = tokio::fs::remove_file(&target).await;
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(format!("rename partial: {e}"));
    }
    if crash_after == Some(CrashAfter::Rename) {
        return Ok(false);
    }

    // Step 3: point the record at the moved file.
    let mut moved = meta.clone();
    moved.path = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    moved.file = Some(target.to_string_lossy().into_owned());
    moved.file_root = Some(files_dir.to_string_lossy().into_owned());
    if let Err(e) = save_artifact_meta(workspace_dir, &moved).await {
        let _ = tokio::fs::remove_file(&target).await;
        return Err(format!("write meta: {e}"));
    }
    if crash_after == Some(CrashAfter::MetaWrite) {
        return Ok(false);
    }

    // Step 4: the record no longer points at the legacy bytes.
    if let Err(e) = tokio::fs::remove_file(src).await {
        log::warn!(
            "[artifacts][migrate] id={} moved; legacy copy not removed: {e}",
            meta.id
        );
    }
    log::debug!("[artifacts][migrate] id={} moved to files folder", meta.id);
    Ok(true)
}

/// Remove `.<id>.partial` leftovers from an interrupted earlier run.
async fn remove_stale_partials(files_dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(files_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name.ends_with(PARTIAL_SUFFIX) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

#[cfg(test)]
#[path = "migrate_tests.rs"]
mod tests;
