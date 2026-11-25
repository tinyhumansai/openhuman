//! The memory domain's small runtime state values (`jobs`, `channel_threads`,
//! `pending_deletions`, `sources_state`, `import_state`,
//! `conversations_backfill`, `layout_migration`, `local_root`) on the storage
//! port.
//!
//! Each used to be `<workspace>/memory/<name>.json`. Each is now one
//! `tsd.kv` entry (owner `memory`, namespace `state`, key `<name>`) in the
//! acting scope; pending deletions are the exception, stored in a stable
//! workspace scope so retries survive the end of profile context. The first
//! read in a backend imports the legacy file once ([`StateKv::import_file`])
//! and leaves it in place.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::Digest;

use crate::storage::state_kv::StateKv;

const OWNER: &str = "memory";
const NS: &str = "state";
const QUEUE_MIGRATION_PREFIX: &str = "pending_deletions_scope_migration_v1";

/// The legacy file of state value `name` in `workspace_dir`.
#[must_use]
pub(crate) fn legacy_path(workspace_dir: &Path, name: &str) -> PathBuf {
    workspace_dir.join("memory").join(format!("{name}.json"))
}

fn open(workspace_dir: &Path, name: &str) -> Result<StateKv> {
    let kv = StateKv::open(workspace_dir, OWNER, NS)?;
    kv.import_file(name, &legacy_path(workspace_dir, name))?;
    Ok(kv)
}

fn open_quarantining<T: DeserializeOwned>(workspace_dir: &Path, name: &str) -> Result<StateKv> {
    let kv = StateKv::open(workspace_dir, OWNER, NS)?;
    kv.import_file_quarantining::<T>(name, &legacy_path(workspace_dir, name))?;
    Ok(kv)
}

fn profile_layout_for_workspace(
    workspace_dir: &Path,
) -> Option<crate::config::schema::ProfileLayout> {
    use crate::config::schema::profile_layout::{PROFILE_WORKSPACE_DIR, USERS_DIR};

    if let Some(profile) = crate::config::schema::ProfileLayout::of_workspace(workspace_dir) {
        return Some(profile);
    }
    if workspace_dir.file_name()? != std::ffi::OsStr::new(PROFILE_WORKSPACE_DIR) {
        return None;
    }
    let profile_dir = workspace_dir.parent()?;
    if profile_dir.parent()?.file_name()? != std::ffi::OsStr::new(USERS_DIR) {
        return None;
    }
    Some(crate::config::schema::ProfileLayout::at(
        profile_dir.to_path_buf(),
    ))
}

fn stable_workspace_identity(workspace_dir: &Path) -> Result<PathBuf> {
    // Layout v1 and v2 use `workspace/` and `files/` under the same profile.
    // Hash the canonical profile directory when known, so a layout migration
    // does not change the queue's scope. For ordinary/custom workspaces use a
    // lexically normalized absolute path; filesystem canonicalization would
    // change identity when a not-yet-created workspace first appears.
    let path = profile_layout_for_workspace(workspace_dir)
        .map(|profile| profile.dir)
        .unwrap_or_else(|| workspace_dir.to_path_buf());
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

fn workspace_scope(workspace_dir: &Path) -> Result<crate::storage::Scope> {
    let path = stable_workspace_identity(workspace_dir)?;
    let digest = sha2::Sha256::digest(path.to_string_lossy().as_bytes());
    let suffix = digest
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(crate::storage::scope_for_module(
        None,
        &format!("memory-workspace-{suffix}"),
    )?)
}

pub(crate) fn open_workspace_quarantining<T: DeserializeOwned>(
    workspace_dir: &Path,
    name: &str,
) -> Result<StateKv> {
    // Workspace state uses conditional writes for safe legacy import and
    // updates. The ordinary unit-test backend is in-memory and does not
    // implement that capability, so use the durable workspace default just
    // as `StateKv::open` does for the acting scope.
    #[cfg(all(test, feature = "storage-sqlite"))]
    crate::storage::local::use_persistent_default_for_test(workspace_dir);

    let scope = workspace_scope(workspace_dir)?;
    let backend = crate::storage::local::backend_for(workspace_dir)?;
    open_workspace_quarantining_on::<T>(workspace_dir, &backend, &scope, name)
}

fn open_workspace_quarantining_on<T: DeserializeOwned>(
    workspace_dir: &Path,
    backend: &std::sync::Arc<dyn crate::storage::StorageBackend>,
    scope: &crate::storage::Scope,
    name: &str,
) -> Result<StateKv> {
    let kv = StateKv::on_backend(backend, scope, OWNER, NS)?;
    kv.import_file_quarantining::<T>(name, &legacy_path(workspace_dir, name))?;
    if name == "pending_deletions" {
        migrate_existing_queue_scopes(
            workspace_dir,
            backend,
            scope,
            &kv,
            active_workspace_matches(workspace_dir),
        )?;
    }
    Ok(kv)
}

fn active_workspace_matches(workspace_dir: &Path) -> bool {
    let dispatch = crate::core::runtime::DispatchCtx::ambient();
    let active_workspace = dispatch
        .resolved()
        .and_then(|context| context.workspace_dir().ok());
    active_workspace
        .as_deref()
        .and_then(|active| stable_workspace_identity(active).ok())
        .zip(stable_workspace_identity(workspace_dir).ok())
        .is_some_and(|(active, requested)| active == requested)
}

fn legacy_queue_scopes(workspace_dir: &Path, include_local: bool) -> Vec<crate::storage::Scope> {
    let known_profile = profile_layout_for_workspace(workspace_dir)
        .and_then(|profile| profile.dir.file_name()?.to_str().map(str::to_owned));
    let mut scopes = Vec::new();
    if include_local {
        scopes.push(crate::storage::Scope::local());
    }
    if let Some(profile) = &known_profile {
        scopes.push(crate::storage::scope_for_profile(profile));
    }

    // A task may still carry the old profile/agent scope for a custom
    // workspace that the profile-layout helper cannot identify. Only inspect
    // that task's own scope, and when the path did identify a profile require
    // it to match; never enumerate other tenants on a shared backend.
    let dispatch = crate::core::runtime::DispatchCtx::ambient();
    if active_workspace_matches(workspace_dir) {
        if let Ok(tenant) = dispatch.tenant() {
            match (tenant.profile, tenant.agent) {
                (Some(profile), _)
                    if known_profile.as_ref().is_none_or(|known| known == &profile) =>
                {
                    scopes.push(crate::storage::scope_for_profile(&profile));
                }
                (None, Some(agent)) if known_profile.is_none() => {
                    scopes.push(crate::storage::scope_for_agent(&agent));
                }
                _ => {}
            }
        }
    }

    scopes.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    scopes.dedup();
    scopes
}

fn queue_migration_marker(scope: &crate::storage::Scope) -> String {
    let digest = sha2::Sha256::digest(scope.as_str().as_bytes());
    let suffix = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{QUEUE_MIGRATION_PREFIX}_{suffix}")
}

fn migrate_existing_queue_scopes(
    workspace_dir: &Path,
    backend: &std::sync::Arc<dyn crate::storage::StorageBackend>,
    target_scope: &crate::storage::Scope,
    target: &StateKv,
    include_local: bool,
) -> Result<()> {
    // The per-source marker records that migration has run, but each queue
    // access still checks the eligible source value. A concurrently running
    // older host may append after a pass; the next access must see that owed
    // work. Once removed, a normally drained value is absent from the source,
    // so it cannot be resurrected after a restart.
    for source_scope in legacy_queue_scopes(workspace_dir, include_local) {
        if &source_scope == target_scope {
            continue;
        }
        let marker = queue_migration_marker(&source_scope);
        let source = StateKv::on_backend(backend, &source_scope, OWNER, NS)?;
        let Some(previous) =
            source.get::<Vec<super::deletion::PendingDeletion>>("pending_deletions")?
        else {
            if target.get::<String>(&marker)?.as_deref() != Some("complete") {
                target.put(&marker, &"complete")?;
            }
            continue;
        };
        if previous.is_empty() {
            if target.get::<String>(&marker)?.as_deref() != Some("complete") {
                target.put(&marker, &"complete")?;
            }
            continue;
        }
        let transferred = previous.clone();
        target.update::<Vec<super::deletion::PendingDeletion>, _>(
            "pending_deletions",
            move |pending| {
                let mut changed = false;
                for deletion in &previous {
                    if !pending.contains(deletion) {
                        pending.push(deletion.clone());
                        changed = true;
                    }
                }
                ((), changed)
            },
        )?;
        source.update::<Vec<super::deletion::PendingDeletion>, _>(
            "pending_deletions",
            move |pending| {
                let before = pending.len();
                pending.retain(|deletion| !transferred.contains(deletion));
                let changed = before != pending.len();
                ((), changed)
            },
        )?;
        target.put(&marker, &"complete")?;
    }
    Ok(())
}

/// A workspace-owned state value in a deterministic local module scope. Use
/// this for queues that must remain readable after the ambient profile scope
/// has ended, while remaining isolated from other workspaces on shared storage.
pub(crate) fn load_workspace<T: DeserializeOwned + Send + 'static>(
    workspace_dir: &Path,
    name: &str,
) -> Result<Option<T>> {
    open_workspace_quarantining::<T>(workspace_dir, name)?.get(name)
}

/// Stores a workspace-owned state value in its deterministic local module scope.
pub(crate) fn save_workspace<T>(workspace_dir: &Path, name: &str, value: &T) -> Result<()>
where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    open_workspace_quarantining::<T>(workspace_dir, name)?.put(name, value)
}

/// Removes a workspace-owned state value from its deterministic local module scope.
pub(crate) fn remove_workspace<T: DeserializeOwned + Send + 'static>(
    workspace_dir: &Path,
    name: &str,
) -> Result<()> {
    open_workspace_quarantining::<T>(workspace_dir, name)?.delete(name)?;
    Ok(())
}

/// [`open`] for a value a damaged legacy file must not reset.
fn open_strict(workspace_dir: &Path, name: &str) -> Result<StateKv> {
    let kv = StateKv::open(workspace_dir, OWNER, NS)?;
    kv.import_file_strict(name, &legacy_path(workspace_dir, name))?;
    Ok(kv)
}

/// Imports every `<workspace>/memory/*.json` state file once into the local
/// scope, except the pending-deletion queue, which is imported into its
/// workspace-owned module scope. For the layout migration (`storage::layout`),
/// which deletes those files afterwards; a file that is not JSON is an error
/// here (nothing may be deleted unimported), so it is imported strictly.
/// Returns the names imported.
///
/// # Errors
///
/// A storage error or an unparsable file.
pub(crate) fn import_all(
    backend: &std::sync::Arc<dyn crate::storage::StorageBackend>,
    workspace_dir: &Path,
) -> Result<Vec<String>> {
    let local_kv = StateKv::on_backend(backend, &crate::storage::Scope::local(), OWNER, NS)?;
    let queue_scope = workspace_scope(workspace_dir)?;
    let queue_kv = StateKv::on_backend(backend, &queue_scope, OWNER, NS)?;
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(workspace_dir.join("memory")) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .and_then(|f| f.to_str())
                .and_then(|f| f.strip_suffix(".json"))
                .map(str::to_owned)
            else {
                continue;
            };
            if !path.is_file() {
                continue;
            }
            if name == "pending_deletions" {
                queue_kv.import_file_strict(&name, &path)?;
            } else {
                local_kv.import_file_strict(&name, &path)?;
            }
            names.push(name);
        }
    }
    migrate_existing_queue_scopes(workspace_dir, backend, &queue_scope, &queue_kv, true)?;
    names.sort();
    Ok(names)
}

/// [`load`], failing on a damaged legacy file instead of starting over.
///
/// # Errors
///
/// A storage error, an unparsable legacy file, or a stored value that is not
/// a `T`.
pub(crate) fn load_strict<T: DeserializeOwned + Send + 'static>(
    workspace_dir: &Path,
    name: &str,
) -> Result<Option<T>> {
    open_strict(workspace_dir, name)?.get(name)
}

/// State value `name`, or `None` when none was stored.
///
/// # Errors
///
/// A storage error, or a stored value that is not a `T`.
pub(crate) fn load<T: DeserializeOwned + Send + 'static>(
    workspace_dir: &Path,
    name: &str,
) -> Result<Option<T>> {
    open_quarantining::<T>(workspace_dir, name)?.get(name)
}

/// The names of the state values starting with `prefix`: those stored, plus
/// those whose legacy file is still in `<workspace>/memory/` (each is
/// imported first, so a name appears once).
///
/// # Errors
///
/// A storage error, or an unparsable legacy file.
pub(crate) fn names(workspace_dir: &Path, prefix: &str) -> Result<Vec<String>> {
    if let Ok(entries) = std::fs::read_dir(workspace_dir.join("memory")) {
        for entry in entries.flatten() {
            let file = entry.file_name();
            let Some(name) = file
                .to_str()
                .and_then(|file| file.strip_suffix(".json"))
                .filter(|name| name.starts_with(prefix))
            else {
                continue;
            };
            open_strict(workspace_dir, name)?;
        }
    }
    let kv = StateKv::open(workspace_dir, OWNER, NS)?;
    Ok(kv
        .list::<serde_json::Value>(prefix)?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

/// Stores `value` as state value `name`.
///
/// # Errors
///
/// A storage error.
pub(crate) fn save<T: Serialize>(workspace_dir: &Path, name: &str, value: &T) -> Result<()> {
    open(workspace_dir, name)?.put(name, value)
}

/// Removes state value `name`.
///
/// # Errors
///
/// A storage error.
pub(crate) fn remove(workspace_dir: &Path, name: &str) -> Result<()> {
    open(workspace_dir, name)?.delete(name).map(|_| ())
}

/// Stores `value` as `name` only when none exists, and returns what is stored.
///
/// # Errors
///
/// A storage error.
pub(crate) fn record_once<T>(workspace_dir: &Path, name: &str, value: &T) -> Result<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + 'static,
{
    let kv = open_strict(workspace_dir, name)?;
    if kv.insert_if_absent(name, value)? {
        return Ok(value.clone());
    }
    kv.get(name)?
        .ok_or_else(|| anyhow::anyhow!("the {name} record vanished"))
}

#[cfg(test)]
#[path = "state_store_tests.rs"]
mod tests;
