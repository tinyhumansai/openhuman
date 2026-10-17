//! The orchestrator: the seven steps of the module docs, resumable.

use std::path::{Path, PathBuf};

use super::archive::{self, Source};
use super::plan::{self, Action, Plan};
use super::state::{LayoutError, LayoutState, Lock, Step};
use super::{db, legacy, DryRun, Noop, Outcome, Report, Target};
use crate::config::schema::profile_layout::{LayoutVersion, PROFILE_LAYOUT_MARKER};
use crate::config::Config;

/// How a migration runs.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Plan only; write nothing.
    pub dry_run: bool,
    /// `OPENHUMAN_STORAGE_URL`, read by the caller (so the rule is testable
    /// without touching the process environment).
    pub env_url: Option<String>,
    /// Test hook: stop with [`LayoutError::InjectedCrash`] after this step's
    /// work, before it is recorded.
    pub crash_after: Option<Step>,
    /// Skip the check that this process does not serve the v1 database.
    pub allow_live: bool,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn crash(options: &Options, step: Step) -> Result<(), LayoutError> {
    if options.crash_after == Some(step) {
        tracing::warn!(target: "openhuman::storage", ?step, "[layout] injected crash");
        return Err(LayoutError::InjectedCrash(step));
    }
    Ok(())
}

/// Migrates `target`'s profile (see the module docs of [`super`]).
///
/// # Errors
///
/// [`LayoutError::Locked`] when another migrator runs, [`LayoutError::Live`]
/// when this process serves the v1 database, a failed verification, or an I/O
/// error. The profile is left resumable; running again continues.
pub async fn migrate(
    target: &Target,
    config: &Config,
    options: &Options,
) -> Result<Outcome, LayoutError> {
    if crate::core::runtime::mode::is_saas() {
        return Ok(Outcome::Noop { reason: Noop::Saas });
    }
    if let crate::storage::config::StorageMode::Url(url) =
        crate::storage::config::mode_from(options.env_url.clone(), config)
    {
        if !options.dry_run {
            // The schema migrations are all an explicit backend gets, and
            // opening it runs them.
            crate::storage::open(&url).await.map_err(|e| {
                LayoutError::Other(anyhow::anyhow!("open the configured backend: {e}"))
            })?;
        }
        return Ok(Outcome::Noop {
            reason: Noop::ExplicitUrl,
        });
    }
    let resuming = LayoutState::load(&target.profile_dir)?.is_some();
    if !resuming && LayoutVersion::detect(&target.profile_dir) == LayoutVersion::V2 {
        return Ok(Outcome::Noop {
            reason: Noop::AlreadyV2,
        });
    }
    if options.dry_run {
        return dry_run(target, resuming).map(|d| Outcome::DryRun(Box::new(d)));
    }
    if !options.allow_live {
        if let Some(url) = live_conflict(target, crate::storage::installed_url()) {
            return Err(LayoutError::Live(url));
        }
    }
    let _lock = Lock::acquire(&target.profile_dir)?;
    // The state may have changed between the first look and the lock.
    let mut state = match LayoutState::load(&target.profile_dir)? {
        Some(state) => state,
        None => {
            if LayoutVersion::detect(&target.profile_dir) == LayoutVersion::V2 {
                return Ok(Outcome::Noop {
                    reason: Noop::AlreadyV2,
                });
            }
            let state = LayoutState::new(&target.user_id, now_ms());
            state.save(&target.profile_dir)?;
            state
        }
    };
    tracing::info!(
        target: "openhuman::storage",
        user = %target.user_id,
        done = ?state.done,
        "[layout] migration starting"
    );
    let mut steps_run = Vec::new();
    let mut database_rows = None;
    let mut archive_files = 0u64;

    for step in Step::ALL {
        if state.has(step) {
            continue;
        }
        tracing::debug!(target: "openhuman::storage", ?step, "[layout] step starting");
        match step {
            Step::Import => {
                legacy::import(target, config).await?;
            }
            Step::Migrate => {
                legacy::migrate(target).await?;
                let scanned = plan::scan(&target.workspace())?;
                state.plan = Some(scanned);
            }
            Step::CopyDb => {
                let source = legacy::v1_db(target);
                if source.is_file() {
                    let source2 = source.clone();
                    let dest = target.storage_db();
                    let copied = tokio::task::spawn_blocking(move || db::copy(&source2, &dest))
                        .await
                        .map_err(|e| LayoutError::Other(e.into()))??;
                    database_rows = Some(copied.counts.values().sum());
                }
            }
            Step::MoveFiles => {
                let plan = plan_of(&state)?.clone();
                move_files(target, &plan)?;
            }
            Step::Archive => {
                let plan = plan_of(&state)?.clone();
                archive_files = write_archive(target, &state, &plan).await?;
            }
            Step::Finish => {
                let plan = plan_of(&state)?.clone();
                finish(target, &state, &plan)?;
            }
        }
        crash(options, step)?;
        steps_run.push(step);
        if step == Step::Finish {
            // The state file goes last: its absence says "finished".
            LayoutState::clear(&target.profile_dir)?;
        } else {
            state.finish(step, &target.profile_dir)?;
        }
    }

    let plan = plan_of(&state)?;
    Ok(Outcome::Migrated(Box::new(Report {
        archive: target.backups().join(&state.archive),
        archive_files,
        moved: names(plan, Action::Move),
        deleted: names(plan, Action::Delete),
        unknown: names(plan, Action::Unknown),
        database_rows,
        steps_run,
        restart_required: true,
    })))
}

/// The installed backend's URL when it is `target`'s v1 database: this
/// process is writing to what the migration is about to snapshot.
fn live_conflict(target: &Target, installed_url: Option<String>) -> Option<String> {
    installed_url
        .filter(|url| *url == legacy::v1_url(target))
        .map(|url| crate::config::schema::storage::redact_url(&url))
}

fn plan_of(state: &LayoutState) -> Result<&Plan, LayoutError> {
    state.plan.as_ref().ok_or_else(|| {
        LayoutError::Other(anyhow::anyhow!(
            "the layout state holds no plan; remove .layout.state to start over"
        ))
    })
}

fn names(plan: &Plan, action: Action) -> Vec<String> {
    plan.with(action).map(|e| e.path.clone()).collect()
}

fn dry_run(target: &Target, resuming: bool) -> Result<DryRun, LayoutError> {
    let state = LayoutState::load(&target.profile_dir)?;
    let scanned = match state.as_ref().and_then(|s| s.plan.clone()) {
        Some(plan) => plan,
        None => plan::scan(&target.workspace())?,
    };
    let database_bytes = std::fs::metadata(legacy::v1_db(target))
        .ok()
        .map(|m| m.len());
    Ok(DryRun {
        archive: scanned.archive_totals(),
        move_entries: scanned.with(Action::Move).cloned().collect(),
        delete_entries: scanned.with(Action::Delete).cloned().collect(),
        unknown_entries: scanned.with(Action::Unknown).cloned().collect(),
        move_totals: scanned.totals(Action::Move),
        delete_totals: scanned.totals(Action::Delete),
        unknown_totals: scanned.totals(Action::Unknown),
        database_bytes,
        archive_dir: target.backups(),
        resuming,
    })
}

/// Step 5. Moves each `Move` entry from `workspace/<rel>` to `files/<rel>`.
fn move_files(target: &Target, plan: &Plan) -> Result<(), LayoutError> {
    let (workspace, files) = (target.workspace(), target.files());
    std::fs::create_dir_all(&files)?;
    for entry in plan.with(Action::Move) {
        let from = plan::join(&workspace, &entry.path);
        let to = plan::join(&files, &entry.path);
        if std::fs::symlink_metadata(&from).is_err() {
            continue; // already moved
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        move_entry(&from, &to)?;
        tracing::debug!(target: "openhuman::storage", entry = %entry.path, "[layout] moved");
    }
    Ok(())
}

/// Renames `from` to `to`; when `to` exists and both are directories, merges
/// child by child. A file already at `to` is a conflict, never overwritten.
fn move_entry(from: &Path, to: &Path) -> Result<(), LayoutError> {
    match std::fs::symlink_metadata(to) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::rename(from, to)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
        Ok(existing) => {
            let from_meta = std::fs::symlink_metadata(from)?;
            if existing.is_dir() && from_meta.is_dir() {
                for child in std::fs::read_dir(from)? {
                    let child = child?;
                    move_entry(&child.path(), &to.join(child.file_name()))?;
                }
                std::fs::remove_dir(from)?;
                Ok(())
            } else {
                Err(LayoutError::Conflict(format!(
                    "{} already exists; nothing was overwritten",
                    to.display()
                )))
            }
        }
    }
}

/// Step 6. Archives, verifies and publishes the archive; returns its file count.
async fn write_archive(
    target: &Target,
    state: &LayoutState,
    plan: &Plan,
) -> Result<u64, LayoutError> {
    let final_path = target.backups().join(&state.archive);
    if final_path.is_file() {
        let path = final_path.clone();
        let manifest = tokio::task::spawn_blocking(move || archive::verify(&path))
            .await
            .map_err(|e| LayoutError::Other(e.into()))??;
        return Ok(manifest.files.len() as u64);
    }
    // The moved entries are in files/, the rest still in workspace/.
    let (workspace, files) = (target.workspace(), target.files());
    let mut sources = Vec::new();
    for entry in plan.entries.iter().filter(|e| e.action != Action::Unknown) {
        let moved = plan::join(&files, &entry.path);
        let at = if entry.action == Action::Move && std::fs::symlink_metadata(&moved).is_ok() {
            moved
        } else {
            plan::join(&workspace, &entry.path)
        };
        if std::fs::symlink_metadata(&at).is_ok() {
            sources.push(Source {
                rel: entry.path.clone(),
                action: entry.action,
                at,
            });
        }
    }
    // The database about to be deleted must be whole in its `.db` file.
    let v1 = legacy::v1_db(target);
    if v1.is_file() {
        db::checkpoint(&v1)?;
    }
    let tmp = final_path.with_file_name(format!("{}.tmp", state.archive));
    let (user, created) = (target.user_id.clone(), state.started_ms);
    let written = tmp.clone();
    let manifest =
        tokio::task::spawn_blocking(move || -> Result<archive::Manifest, LayoutError> {
            archive::write(&written, &user, created, &sources)?;
            archive::verify(&written)
        })
        .await
        .map_err(|e| LayoutError::Other(e.into()))??;
    std::fs::rename(&tmp, &final_path)?;
    // The sidecar was written beside the temp name; give it the final name.
    let _ = std::fs::rename(
        archive::sidecar_path(&tmp),
        archive::sidecar_path(&final_path),
    );
    Ok(manifest.files.len() as u64)
}

/// Step 7. Marker first, then the deletions, so a crash between them leaves
/// nothing worse than leftovers that the resumed run removes.
fn finish(target: &Target, state: &LayoutState, plan: &Plan) -> Result<(), LayoutError> {
    let archive = target.backups().join(&state.archive);
    if !archive.is_file() || !state.has(Step::Archive) {
        return Err(LayoutError::Verify(
            "no verified archive; refusing to delete".to_owned(),
        ));
    }
    if legacy::v1_db(target).is_file() && !target.storage_db().is_file() {
        return Err(LayoutError::Verify(
            "no verified storage.db; refusing to delete".to_owned(),
        ));
    }
    std::fs::create_dir_all(target.files())?;
    LayoutVersion::mark_v2(&target.profile_dir)?;
    let workspace = target.workspace();
    for entry in plan.with(Action::Delete) {
        remove_within(&workspace, &plan::join(&workspace, &entry.path))?;
    }
    // Directories the deletions emptied.
    let memory = workspace.join("memory");
    let _ = std::fs::remove_dir(&memory);
    let _ = std::fs::remove_dir(&workspace);
    tracing::info!(
        target: "openhuman::storage",
        user = %target.user_id,
        unknown = plan.totals(Action::Unknown).entries,
        "[layout] migration finished"
    );
    Ok(())
}

/// Removes `path`, which must lie below `base`, without following symlinks.
fn remove_within(base: &Path, path: &Path) -> Result<(), LayoutError> {
    // `starts_with` is lexical, so `base/../x` would pass it: every component
    // below `base` must be a plain name (a restore reads entry paths from a
    // caller-supplied archive).
    let below = path.strip_prefix(base).ok().filter(|rel| {
        rel.components().next().is_some()
            && rel
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
    });
    if below.is_none() {
        return Err(LayoutError::Verify(format!(
            "refusing to delete {}",
            path.display()
        )));
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path)?,
        Ok(_) => std::fs::remove_file(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// Puts a migrated profile back to layout v1 from `archive`.
///
/// Verifies the archive, extracts the entries under `workspace/`, sets the
/// layout-v2 `storage.db` aside as `storage.v2-<ms>.db` (it is never deleted),
/// removes the moved entries from `files/`, and removes the marker. Takes the
/// same lock as a migration and refuses while one is in progress.
///
/// # Errors
///
/// A failed verification (nothing is changed then), a held lock, or an I/O
/// error. A failure after extraction is safe to retry.
pub async fn restore(target: &Target, archive_path: &Path) -> Result<RestoreReport, LayoutError> {
    let _lock = Lock::acquire(&target.profile_dir)?;
    if LayoutState::load(&target.profile_dir)?.is_some() {
        return Err(LayoutError::Other(anyhow::anyhow!(
            "a migration is in progress; finish it before restoring"
        )));
    }
    let (archive_path, profile) = (archive_path.to_path_buf(), target.profile_dir.clone());
    let manifest = tokio::task::spawn_blocking(move || archive::restore(&archive_path, &profile))
        .await
        .map_err(|e| LayoutError::Other(e.into()))??;
    let files = target.files();
    for entry in manifest.entries.iter().filter(|e| e.action == Action::Move) {
        remove_within(&files, &plan::join(&files, &entry.path))?;
    }
    let mut set_aside = None;
    let db = target.storage_db();
    if db.is_file() {
        let aside = target
            .profile_dir
            .join(format!("storage.v2-{}.db", now_ms()));
        std::fs::rename(&db, &aside)?;
        for suffix in ["-wal", "-shm"] {
            let mut name = db.file_name().unwrap_or_default().to_os_string();
            name.push(suffix);
            let _ = std::fs::remove_file(db.with_file_name(name));
        }
        set_aside = Some(aside);
    }
    match std::fs::remove_file(target.profile_dir.join(PROFILE_LAYOUT_MARKER)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(RestoreReport {
        entries: manifest.entries.len(),
        files: manifest.files.len() as u64,
        v2_database: set_aside,
        restart_required: true,
    })
}

/// What [`restore`] did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RestoreReport {
    /// Top-level entries restored.
    pub entries: usize,
    /// Files restored.
    pub files: u64,
    /// Where the layout-v2 database was set aside.
    pub v2_database: Option<PathBuf>,
    /// A running core still serves the v2 layout until it restarts.
    pub restart_required: bool,
}

/// The boot hook behind `[storage] auto_migrate_layout`: migrates the active
/// profile when `config` opts in, before any backend opens. Returns whether a
/// migration ran (the caller reloads its config then). A failure is logged and
/// the profile stays on v1, which keeps working.
pub async fn auto_migrate(config: &Config) -> bool {
    if !config.storage.auto_migrate_layout {
        return false;
    }
    let Some(target) = Target::from_config(config) else {
        return false;
    };
    let options = Options {
        env_url: std::env::var(crate::storage::STORAGE_URL_VAR).ok(),
        ..Options::default()
    };
    match migrate(&target, config, &options).await {
        Ok(Outcome::Migrated(report)) => {
            tracing::info!(
                target: "openhuman::storage",
                archive = %report.archive.display(),
                "[layout] auto-migrated the profile to layout v2"
            );
            true
        }
        Ok(_) => false,
        Err(error) => {
            tracing::warn!(
                target: "openhuman::storage",
                %error,
                "[layout] auto-migration failed; the profile stays on layout v1 and retries at the next boot"
            );
            false
        }
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
