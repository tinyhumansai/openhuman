//! Session storage layout migration: date-grouped → flat `session_raw/`.
//!
//! Older releases (≤ 0.53.4) wrote transcripts to:
//!
//! ```text
//! {workspace}/session_raw/{DDMMYYYY}/{stem}.jsonl
//! {workspace}/sessions/{DDMMYYYY}/{stem}.md
//! ```
//!
//! From 0.53.5 onwards the source of truth is the *flat*
//! `session_raw/{stem}.jsonl` and the human-readable companion is
//! `sessions/{YYYY_MM_DD}/{stem}.md` — see
//! [`super::transcript`] for the rationale (idle-thread resume
//! becomes date-independent).
//!
//! `find_latest_transcript` ships a fallback that reads the legacy
//! layout when the flat dir is empty, so users upgrading don't lose
//! resume even before this migration runs. This module performs the
//! one-shot move so files end up in their canonical location and the
//! transitional fallback can eventually be removed.
//!
//! ## Idempotency
//!
//! After a successful migration we write a marker at
//! `{workspace}/state/migrations/session_layout_v1.done`. Subsequent
//! starts read the marker and skip the scan entirely. If the workspace
//! has no legacy layout (fresh install or already migrated by an
//! external sync) we still write the marker so the scan stays
//! single-cost.
//!
//! ## Version gate
//!
//! The marker doubles as the "have we already migrated past 0.53.4?"
//! flag. A bare workspace with no legacy dirs and no marker is treated
//! as "fresh — nothing to do, write the marker." A workspace with
//! legacy dirs is treated as "upgrading from ≤ 0.53.4 — migrate then
//! write the marker."
//!
//! Failures are surfaced as warnings (logged) and **never panic**:
//! transcript files are valuable but not strictly required for
//! continued operation, and an uncatchable migration error would brick
//! every startup.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Marker file that signals "the v1 session-layout migration ran
/// successfully on this workspace at least once". Written under
/// `state/migrations/` to keep the workspace root tidy.
const MIGRATION_MARKER: &str = "state/migrations/session_layout_v1.done";

#[derive(Debug, Default, Clone)]
pub struct MigrationOutcome {
    pub jsonl_moved: usize,
    pub jsonl_skipped: usize,
    pub md_moved: usize,
    pub md_skipped: usize,
    pub legacy_dirs_pruned: usize,
    pub already_done: bool,
    pub warnings: Vec<String>,
}

/// Migrate the session storage layout for `workspace_dir` if needed.
///
/// * Detects legacy `session_raw/{DDMMYYYY}/...jsonl` and
///   `sessions/{DDMMYYYY}/...md` layouts (i.e. an upgrade from
///   ≤ 0.53.4).
/// * Moves jsonl files to flat `session_raw/{stem}.jsonl`.
/// * Renames `DDMMYYYY` md dirs to ISO-style `YYYY_MM_DD` so the
///   listing sorts lexicographically.
/// * Writes the migration marker on success.
///
/// Idempotent: returns immediately with `already_done = true` if the
/// marker already exists. Best-effort on individual file moves —
/// failures are logged and surfaced via `warnings`, not propagated, so
/// one bad rename can't brick startup.
pub fn migrate_session_layout_if_needed(workspace_dir: &Path) -> Result<MigrationOutcome> {
    let marker_path = workspace_dir.join(MIGRATION_MARKER);
    if marker_path.exists() {
        log::debug!(
            "[session-migration] marker present at {} — skipping",
            marker_path.display()
        );
        return Ok(MigrationOutcome {
            already_done: true,
            ..Default::default()
        });
    }

    let mut outcome = MigrationOutcome::default();

    let raw_root = workspace_dir.join("session_raw");
    if raw_root.is_dir() {
        migrate_raw_jsonl(&raw_root, &mut outcome)?;
    }

    let sessions_root = workspace_dir.join("sessions");
    if sessions_root.is_dir() {
        migrate_md_directories(&sessions_root, &mut outcome)?;
    }

    write_marker(&marker_path, &outcome).context("write session-migration marker")?;

    log::info!(
        "[session-migration] complete: jsonl moved={} skipped={}, md moved={} skipped={}, legacy dirs pruned={}, warnings={}",
        outcome.jsonl_moved,
        outcome.jsonl_skipped,
        outcome.md_moved,
        outcome.md_skipped,
        outcome.legacy_dirs_pruned,
        outcome.warnings.len(),
    );

    Ok(outcome)
}

/// Walk `session_raw/`, find direct subdirectories whose names look
/// like `DDMMYYYY` (8 ascii digits), and move every `*.jsonl` file
/// inside up to the flat `session_raw/` parent. Empty legacy dirs
/// are removed; non-empty ones are left in place with a warning so a
/// human can decide what to do.
fn migrate_raw_jsonl(raw_root: &Path, outcome: &mut MigrationOutcome) -> Result<()> {
    let entries = match fs::read_dir(raw_root) {
        Ok(it) => it,
        Err(err) => {
            outcome
                .warnings
                .push(format!("read_dir({}) failed: {err}", raw_root.display()));
            return Ok(());
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_ddmmyyyy(name) {
            // Not a legacy date dir — leave alone (could be the new
            // flat layout's own files which would never be a dir, or
            // a user-created subdirectory we shouldn't touch).
            continue;
        }
        move_jsonl_files_up(&path, raw_root, outcome);
        prune_if_empty(&path, outcome);
    }

    Ok(())
}

fn move_jsonl_files_up(legacy_dir: &Path, flat_dir: &Path, outcome: &mut MigrationOutcome) {
    let entries = match fs::read_dir(legacy_dir) {
        Ok(it) => it,
        Err(err) => {
            outcome
                .warnings
                .push(format!("read_dir({}) failed: {err}", legacy_dir.display()));
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let dest = flat_dir.join(file_name);
        if dest.exists() {
            // Same stem already lives in the flat dir — the new layout
            // is authoritative for current sessions, so leave the
            // legacy copy in place and surface a warning instead of
            // overwriting newer data.
            outcome.jsonl_skipped += 1;
            outcome.warnings.push(format!(
                "skip move: destination already exists at {} (legacy file kept at {})",
                dest.display(),
                path.display()
            ));
            continue;
        }
        match fs::rename(&path, &dest) {
            Ok(()) => {
                outcome.jsonl_moved += 1;
                log::debug!(
                    "[session-migration] moved {} → {}",
                    path.display(),
                    dest.display()
                );
            }
            Err(err) => {
                outcome.warnings.push(format!(
                    "rename({} → {}) failed: {err}",
                    path.display(),
                    dest.display()
                ));
            }
        }
    }
}

/// Walk `sessions/`, rename each `DDMMYYYY` subdirectory to its
/// `YYYY_MM_DD` equivalent. We rename the dir wholesale rather than
/// copying file-by-file: the contents are human-readable companions
/// and don't need re-indexing.
fn migrate_md_directories(sessions_root: &Path, outcome: &mut MigrationOutcome) -> Result<()> {
    let entries = match fs::read_dir(sessions_root) {
        Ok(it) => it,
        Err(err) => {
            outcome.warnings.push(format!(
                "read_dir({}) failed: {err}",
                sessions_root.display()
            ));
            return Ok(());
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(iso) = ddmmyyyy_to_yyyy_mm_dd(name) else {
            continue;
        };
        let dest = sessions_root.join(&iso);
        if dest.exists() {
            // ISO dir already exists — merge file-by-file, never
            // overwrite. A user could have legitimately produced both
            // names (e.g. by manual workflow) so we don't blindly
            // discard either side.
            merge_md_dirs(&path, &dest, outcome);
            prune_if_empty(&path, outcome);
            continue;
        }
        match fs::rename(&path, &dest) {
            Ok(()) => {
                outcome.md_moved += 1;
                log::debug!(
                    "[session-migration] renamed md dir {} → {}",
                    path.display(),
                    dest.display()
                );
            }
            Err(err) => {
                outcome.warnings.push(format!(
                    "rename({} → {}) failed: {err}",
                    path.display(),
                    dest.display()
                ));
            }
        }
    }

    Ok(())
}

fn merge_md_dirs(legacy: &Path, dest: &Path, outcome: &mut MigrationOutcome) {
    let entries = match fs::read_dir(legacy) {
        Ok(it) => it,
        Err(err) => {
            outcome
                .warnings
                .push(format!("read_dir({}) failed: {err}", legacy.display()));
            return;
        }
    };
    for entry in entries.flatten() {
        let src = entry.path();
        if !src.is_file() {
            continue;
        }
        let Some(file_name) = src.file_name() else {
            continue;
        };
        let target = dest.join(file_name);
        if target.exists() {
            outcome.md_skipped += 1;
            outcome.warnings.push(format!(
                "skip md merge: {} already exists (legacy at {} kept)",
                target.display(),
                src.display()
            ));
            continue;
        }
        match fs::rename(&src, &target) {
            Ok(()) => outcome.md_moved += 1,
            Err(err) => outcome.warnings.push(format!(
                "rename({} → {}) failed: {err}",
                src.display(),
                target.display()
            )),
        }
    }
}

fn prune_if_empty(dir: &Path, outcome: &mut MigrationOutcome) {
    match fs::read_dir(dir) {
        Ok(mut it) => {
            if it.next().is_some() {
                // Non-empty — leave for human inspection.
                return;
            }
        }
        Err(_) => return,
    }
    if fs::remove_dir(dir).is_ok() {
        outcome.legacy_dirs_pruned += 1;
    }
}

fn write_marker(marker_path: &Path, outcome: &MigrationOutcome) -> Result<()> {
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create marker dir {}", parent.display()))?;
    }
    let body = format!(
        "openhuman session_layout migration v1\nrun_at: {}\njsonl_moved: {}\nmd_moved: {}\nlegacy_dirs_pruned: {}\nwarnings: {}\n",
        chrono::Utc::now().to_rfc3339(),
        outcome.jsonl_moved,
        outcome.md_moved,
        outcome.legacy_dirs_pruned,
        outcome.warnings.len(),
    );
    fs::write(marker_path, body)
        .with_context(|| format!("write marker {}", marker_path.display()))?;
    Ok(())
}

/// Returns true iff `name` is exactly 8 ASCII digits — the legacy
/// `DDMMYYYY` shape. We don't validate the date range (1–31, 1–12,
/// 1900–2100) because chrono printed the value originally, so any
/// real on-disk dir is well-formed; the digit shape is a sufficient
/// fingerprint to distinguish from user-created subdirectories.
fn is_ddmmyyyy(name: &str) -> bool {
    name.len() == 8 && name.chars().all(|c| c.is_ascii_digit())
}

/// Convert `DDMMYYYY` → `YYYY_MM_DD`. Returns `None` if the input
/// isn't 8 digits.
fn ddmmyyyy_to_yyyy_mm_dd(name: &str) -> Option<String> {
    if !is_ddmmyyyy(name) {
        return None;
    }
    let dd = &name[0..2];
    let mm = &name[2..4];
    let yyyy = &name[4..8];
    Some(format!("{yyyy}_{mm}_{dd}"))
}

/// Returns the path of the migration marker for `workspace_dir`.
/// Exposed for tests and CLI tooling that wants to manually re-run
/// the migration (delete the marker, then call
/// [`migrate_session_layout_if_needed`] again).
pub fn marker_path_for(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(MIGRATION_MARKER)
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
