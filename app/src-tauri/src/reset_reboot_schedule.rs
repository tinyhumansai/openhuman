//! Windows-only fallback for `reset_local_data` (issue #1615).
//!
//! When the in-process `remove_dir_all` step fails because a third-party
//! process (anti-virus, file-indexer, sibling OpenHuman window) still holds
//! an open handle inside the `.openhuman` tree, Windows returns
//! `ERROR_SHARING_VIOLATION` (os error 32) / `ERROR_LOCK_VIOLATION` (33)
//! and the user is stuck — see PR #2395 / #1811, which surface a "close all
//! OpenHuman windows" prompt but cannot break a foreign lock.
//!
//! This module walks the still-present sub-tree depth-first and asks the
//! Windows Session Manager to delete each entry at next boot via
//! `MoveFileExW(src, NULL, MOVEFILE_DELAY_UNTIL_REBOOT)`. The session
//! manager requires that directories be empty when boot-time deletion
//! runs, so children are scheduled before their parent.
//!
//! Reference:
//!   https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw
//!
//! Privileges: `MoveFileExW(.., NULL, MOVEFILE_DELAY_UNTIL_REBOOT)` writes
//! to `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\PendingFileRenameOperations`
//! (the boot-time session manager reads from HKLM, not the per-user hive),
//! so the call **may fail for non-administrator users** with `ERROR_ACCESS_DENIED`.
//! That is by design — Microsoft documents the elevation requirement on the
//! `MOVEFILE_DELAY_UNTIL_REBOOT` flag — and the caller in `lib.rs` handles
//! the failure path gracefully: it preserves the original lock error plus
//! the schedule failure reason and falls back to the "close all OpenHuman
//! windows and try again" guidance from PR #2395 / #1811.

#![cfg(target_os = "windows")]

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};

/// Tally of entries handed off to `MoveFileExW`, returned to the caller so
/// it can log and surface (e.g. "scheduled 142 files / 14 dirs for deletion
/// on next reboot") instead of just an opaque "ok".
///
/// `partial` is `true` when the walk aborted mid-tree (e.g. a directory
/// became unreadable, or an individual `MoveFileExW` call failed). In that
/// case `files` / `dirs` represent **only** what was queued before the
/// failure point — useful for support logs to distinguish "everything is
/// queued" from "some of the tree is queued but the rest still needs
/// manual cleanup." Pair with the `Result::Err` returned by
/// [`schedule_path_for_reboot_deletion`] for the cause.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RebootDeletionSchedule {
    pub files: u32,
    pub dirs: u32,
    pub partial: bool,
}

impl RebootDeletionSchedule {
    pub fn total(&self) -> u32 {
        self.files.saturating_add(self.dirs)
    }
}

/// Schedule `path` (and everything under it if it is a directory) for
/// deletion on the next reboot via `MoveFileExW(_, NULL, MOVEFILE_DELAY_UNTIL_REBOOT)`.
///
/// Strategy:
///   * Regular files / symlinks → scheduled directly.
///   * Directories → children scheduled first (depth-first), then the
///     directory itself once its contents are queued.
///
/// `path` not existing on disk yields `Err(RebootDeletionFailure { error: NotFound, .. })` —
/// callers can choose to treat that as a no-op since "nothing to remove" is
/// the same outcome.
///
/// On error the failure carries a partially-populated `RebootDeletionSchedule`
/// (`partial = true`) so the caller can surface "we queued N files and M
/// folders before scheduling failed" instead of just the bare io error.
/// The walk is depth-first, so the counts reflect entries queued *before*
/// the failing step.
pub fn schedule_path_for_reboot_deletion(
    path: &Path,
) -> Result<RebootDeletionSchedule, RebootDeletionFailure> {
    schedule_path_with_scheduler(path, &mut schedule_one)
}

/// Internal seam used by both [`schedule_path_for_reboot_deletion`] (which
/// passes the real `MoveFileExW` step as `scheduler`) and the unit tests
/// (which pass an injectable `Ok(())` stub so the traversal/counting logic
/// can be exercised on every dev machine without needing administrator
/// rights or actually queuing reboot-time deletions).
fn schedule_path_with_scheduler<F>(
    path: &Path,
    scheduler: &mut F,
) -> Result<RebootDeletionSchedule, RebootDeletionFailure>
where
    F: FnMut(&Path) -> io::Result<()>,
{
    let metadata = std::fs::symlink_metadata(path).map_err(|error| RebootDeletionFailure {
        error,
        partial: RebootDeletionSchedule {
            partial: true,
            ..RebootDeletionSchedule::default()
        },
    })?;
    let mut summary = RebootDeletionSchedule::default();
    match schedule_inner(path, &metadata, &mut summary, scheduler) {
        Ok(()) => Ok(summary),
        Err(error) => {
            summary.partial = true;
            Err(RebootDeletionFailure {
                error,
                partial: summary,
            })
        }
    }
}

/// Pair of `(io::Error, partial schedule)` returned when the depth-first
/// walk aborts mid-tree. The `partial` field records what was queued via
/// `MoveFileExW` *before* the failure point so the caller can include the
/// counts in user-facing copy and support logs ("123 files / 7 folders
/// were queued for the next reboot before scheduling failed: <reason>").
#[derive(Debug)]
pub struct RebootDeletionFailure {
    pub error: io::Error,
    pub partial: RebootDeletionSchedule,
}

impl std::fmt::Display for RebootDeletionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for RebootDeletionFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

fn schedule_inner<F>(
    path: &Path,
    metadata: &std::fs::Metadata,
    summary: &mut RebootDeletionSchedule,
    scheduler: &mut F,
) -> io::Result<()>
where
    F: FnMut(&Path) -> io::Result<()>,
{
    // Symlinked directories must NOT be descended into — the lock lives
    // on the link target, not the link itself, and following would queue
    // unrelated paths for deletion. Treat symlinks (file or dir) as a
    // single leaf entry.
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let child_meta = entry.metadata()?;
            schedule_inner(&entry.path(), &child_meta, summary, scheduler)?;
        }
        scheduler(path)?;
        summary.dirs = summary.dirs.saturating_add(1);
    } else {
        scheduler(path)?;
        summary.files = summary.files.saturating_add(1);
    }
    Ok(())
}

fn schedule_one(path: &Path) -> io::Result<()> {
    // `MoveFileExW + MOVEFILE_DELAY_UNTIL_REBOOT` requires absolute paths —
    // the session manager runs at boot before any working directory is
    // established, so a relative path cannot be resolved. The call sites
    // in `reset_local_data` already resolve paths via the core's
    // `config_get_data_paths` RPC (which returns absolute paths) so this
    // is currently a no-op in release builds; the assert catches a future
    // regression that wires a different caller in without thinking.
    debug_assert!(
        path.is_absolute(),
        "MoveFileExW + DELAY_UNTIL_REBOOT requires an absolute path, got {}",
        path.display()
    );
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 buffer that outlives the
    // call. The destination pointer is `NULL`, which (combined with
    // `MOVEFILE_DELAY_UNTIL_REBOOT`) tells Windows to delete (rather than
    // rename) the source at the next boot. `MoveFileExW` returns BOOL —
    // non-zero on success.
    let ok = unsafe { MoveFileExW(wide.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only no-op scheduler. Lets the traversal/counting tests run
    /// in any user context (incl. non-administrator) by sidestepping the
    /// `MoveFileExW + MOVEFILE_DELAY_UNTIL_REBOOT` call that would
    /// otherwise need HKLM write access. We capture the call order so a
    /// regression in depth-first ordering would surface as a wrong path
    /// sequence here, even though the real OS-side scheduling stays
    /// out of the test process.
    fn noop_scheduler(
        captured: &mut Vec<std::path::PathBuf>,
    ) -> impl FnMut(&Path) -> io::Result<()> + '_ {
        move |path: &Path| {
            captured.push(path.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn schedule_walks_files_then_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("reset-target");
        std::fs::create_dir_all(root.join("nested")).expect("mkdir nested");
        std::fs::write(root.join("a.txt"), b"a").expect("write a.txt");
        std::fs::write(root.join("nested").join("b.txt"), b"b").expect("write b.txt");

        let mut captured = Vec::new();
        let summary =
            schedule_path_with_scheduler(&root, &mut noop_scheduler(&mut captured)).expect("walk");

        // root + nested == 2 dirs; a.txt + nested/b.txt == 2 files
        assert_eq!(summary.files, 2, "expected 2 files queued, got {summary:?}");
        assert_eq!(summary.dirs, 2, "expected 2 dirs queued, got {summary:?}");
        assert_eq!(summary.total(), 4);
        assert!(!summary.partial, "Ok must not flag partial");

        // Depth-first: a parent must only appear after all of its children.
        // Track per-path positions in the call order, then assert each
        // directory sits after every entry whose path is rooted inside it.
        let position = |needle: &Path| -> usize {
            captured
                .iter()
                .position(|p| p == needle)
                .unwrap_or_else(|| panic!("missing {} in {captured:?}", needle.display()))
        };
        let root_pos = position(&root);
        let nested_pos = position(&root.join("nested"));
        let a_pos = position(&root.join("a.txt"));
        let b_pos = position(&root.join("nested").join("b.txt"));
        assert!(
            b_pos < nested_pos,
            "b.txt must be scheduled before its parent (nested)"
        );
        assert!(
            nested_pos < root_pos && a_pos < root_pos,
            "nested + a.txt must be scheduled before root"
        );
    }

    #[test]
    fn schedule_single_file_reports_one_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("solo.txt");
        std::fs::write(&file, b"x").expect("write solo.txt");

        let mut captured = Vec::new();
        let summary =
            schedule_path_with_scheduler(&file, &mut noop_scheduler(&mut captured)).expect("walk");

        assert_eq!(
            summary,
            RebootDeletionSchedule {
                files: 1,
                dirs: 0,
                partial: false,
            }
        );
        assert_eq!(captured, vec![file]);
    }

    #[test]
    fn schedule_missing_path_yields_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist");

        let mut captured = Vec::new();
        let failure = schedule_path_with_scheduler(&missing, &mut noop_scheduler(&mut captured))
            .expect_err("missing");
        assert_eq!(failure.error.kind(), io::ErrorKind::NotFound);
        // Nothing scheduled, but partial flag still reports "did not
        // complete" so callers can distinguish from a clean success.
        assert!(failure.partial.partial);
        assert_eq!(failure.partial.files, 0);
        assert_eq!(failure.partial.dirs, 0);
        assert!(
            captured.is_empty(),
            "no scheduling should have been attempted: {captured:?}"
        );
    }

    #[test]
    fn schedule_empty_dir_counts_one_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let empty = dir.path().join("empty-target");
        std::fs::create_dir(&empty).expect("mkdir empty-target");

        let mut captured = Vec::new();
        let summary =
            schedule_path_with_scheduler(&empty, &mut noop_scheduler(&mut captured)).expect("walk");

        assert_eq!(
            summary,
            RebootDeletionSchedule {
                files: 0,
                dirs: 1,
                partial: false,
            }
        );
        assert_eq!(captured, vec![empty]);
    }

    #[test]
    fn schedule_propagates_scheduler_failure_with_partial_counts() {
        // Simulate the non-administrator MoveFileExW failure path: the
        // walk visits children successfully, then the third call (the
        // parent dir) errors out. The Err must carry the leaf counts
        // queued before the failure so the caller can surface "we did
        // get X files scheduled before this hit the registry wall."
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("partial-fail");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("a.txt"), b"a").expect("write a.txt");
        std::fs::write(root.join("b.txt"), b"b").expect("write b.txt");

        let root_path = root.clone();
        let mut count = 0usize;
        let mut scheduler = |path: &Path| -> io::Result<()> {
            count += 1;
            if path == root_path {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated non-admin MoveFileExW failure",
                ))
            } else {
                Ok(())
            }
        };

        let failure =
            schedule_path_with_scheduler(&root, &mut scheduler).expect_err("scheduler failed");
        assert_eq!(failure.error.kind(), io::ErrorKind::PermissionDenied);
        assert!(failure.partial.partial);
        // Both leaf files were scheduled before the parent-dir call failed.
        assert_eq!(failure.partial.files, 2, "got {:?}", failure.partial);
        assert_eq!(failure.partial.dirs, 0, "got {:?}", failure.partial);
        // Sanity: scheduler was called for both leaves + the parent.
        assert_eq!(count, 3);
    }
}
