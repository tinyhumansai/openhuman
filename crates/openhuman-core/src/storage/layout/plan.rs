//! What a V1 profile's `workspace/` holds, and what the migration does with
//! each entry.
//!
//! The plan is computed from disk every time it is needed (a dry run, the
//! archive step, the final delete), never trusted from an earlier run, so a
//! resumed migration sees the directory as it is now. Each top-level entry of
//! `workspace/` (and, for `memory/`, each entry one level down) gets one
//! [`Action`]:
//!
//! - [`Action::Move`]: the entry is still read from `Config.workspace_dir`
//!   under layout v2, which is `files/`, so it is moved there. This is the
//!   user's own files (skills, attachments, artifacts, the prompt files) and
//!   the runtime files no importer covers (`state/`, `integrations/`, ...).
//! - [`Action::Delete`]: an importer copied the entry into the database (or
//!   the entry *is* the old database). It is archived, and deleted only after
//!   the archive verified and the layout marker is written.
//! - [`Action::Unknown`]: nothing here knows the entry. It is left where it
//!   is and listed, never archived or deleted.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

/// What the migration does with an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Moved into `files/`.
    Move,
    /// Archived, then deleted once the layout marker is written.
    Delete,
    /// Left in place and listed.
    Unknown,
}

/// One entry of the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The path relative to `workspace/`, `/` separated.
    pub path: String,
    /// What happens to it.
    pub action: Action,
    /// Why, in a few words (`skills`, `imported into the database`, ...).
    pub reason: String,
    /// Files below the entry (1 for a file).
    pub files: u64,
    /// Bytes below the entry.
    pub bytes: u64,
}

/// Byte and file totals of one action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    /// Entries.
    pub entries: u64,
    /// Files below them.
    pub files: u64,
    /// Bytes below them.
    pub bytes: u64,
}

/// The classified contents of one `workspace/`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Every entry, in path order.
    pub entries: Vec<Entry>,
}

impl Plan {
    /// The entries with `action`.
    pub fn with(&self, action: Action) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.action == action)
    }

    /// Totals of the entries with `action`.
    pub fn totals(&self, action: Action) -> Totals {
        self.with(action).fold(Totals::default(), |mut t, e| {
            t.entries += 1;
            t.files += e.files;
            t.bytes += e.bytes;
            t
        })
    }

    /// Totals of what the archive holds: the moved and the deleted entries.
    pub fn archive_totals(&self) -> Totals {
        let (moved, deleted) = (self.totals(Action::Move), self.totals(Action::Delete));
        Totals {
            entries: moved.entries + deleted.entries,
            files: moved.files + deleted.files,
            bytes: moved.bytes + deleted.bytes,
        }
    }
}

/// Top-level entries moved into `files/`: the user's files and prompt files,
/// and the runtime files the workspace-relative code still reads.
const MOVE: &[(&str, &str)] = &[
    ("skills", "user skills"),
    ("builtin-skills", "skill bundles"),
    ("attachments", "user attachments"),
    ("artifacts", "artifact files"),
    ("Files", "user files"),
    ("agents", "agent definitions"),
    ("agent", "agent prompt overrides"),
    ("workflows", "workflow definitions"),
    ("STYLE.md", "prompt file"),
    ("SOUL.md", "prompt file"),
    ("IDENTITY.md", "prompt file"),
    ("PROFILE.md", "prompt file"),
    ("SYSTEM.md", "prompt file"),
    ("MEMORY.md", "prompt file"),
    (".SOUL.md.builtin-hash", "prompt file"),
    ("state", "runtime state files"),
    (".openhuman", "sub-agent checkpoints"),
    ("integrations", "integration files"),
    ("modules", "module cache"),
    (".tokenjuice", "tokenjuice caches"),
    ("sessions", "session transcripts"),
];

/// Top-level entries an importer copies into the database. `memory/` is
/// classified one level down ([`classify_memory_child`]).
const DELETE: &[(&str, &str)] = &[
    ("storage", "the v1 database (copied into storage.db)"),
    ("session_raw", "imported into the session store"),
    ("session_db", "imported into the session store"),
    ("approval", "imported into the database"),
    ("devices", "imported into the database"),
    ("notifications", "imported into the database"),
    ("task_sources", "imported into the database"),
    ("flows", "imported into the database"),
    ("cron", "imported into the database"),
];

/// The classification of one top-level entry name, `None` when unknown.
pub fn classify_top(name: &str) -> Option<(Action, &'static str)> {
    if let Some((_, why)) = MOVE.iter().find(|(n, _)| *n == name) {
        return Some((Action::Move, why));
    }
    DELETE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, why)| (Action::Delete, *why))
}

/// The classification of `memory/<name>`: the JSON state values and the
/// conversation log are imported into the database; anything else a memory
/// directory carries (claim markers, layout-migration state) is still read
/// from the workspace and moves.
pub fn classify_memory_child(name: &str, is_dir: bool) -> (Action, &'static str) {
    if (!is_dir && name.ends_with(".json")) || (is_dir && name == "conversations") {
        (Action::Delete, "imported into the database")
    } else {
        (Action::Move, "memory runtime files")
    }
}

/// Files and bytes below `path` (1 and its length for a file). Symlinks are
/// counted, never followed.
pub fn measure(path: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    for entry in WalkDir::new(path).follow_links(false).into_iter().flatten() {
        if entry.file_type().is_dir() {
            continue;
        }
        files += 1;
        bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
    }
    (files, bytes)
}

/// Classifies `workspace` (`users/<id>/workspace`). A missing directory is an
/// empty plan.
///
/// # Errors
///
/// When the directory exists and cannot be listed.
pub fn scan(workspace: &Path) -> std::io::Result<Plan> {
    let mut entries = Vec::new();
    let listing = match std::fs::read_dir(workspace) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Plan::default()),
        Err(error) => return Err(error),
    };
    for item in listing {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        let is_dir = item.file_type()?.is_dir();
        if name == "memory" && is_dir {
            scan_memory(&item.path(), &mut entries)?;
            continue;
        }
        let (action, reason) = classify_top(&name).unwrap_or((Action::Unknown, "not recognised"));
        push(&mut entries, name, &item.path(), action, reason);
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Plan { entries })
}

fn scan_memory(dir: &Path, entries: &mut Vec<Entry>) -> std::io::Result<()> {
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        let (action, reason) = classify_memory_child(&name, item.file_type()?.is_dir());
        push(
            entries,
            format!("memory/{name}"),
            &item.path(),
            action,
            reason,
        );
    }
    Ok(())
}

fn push(entries: &mut Vec<Entry>, path: String, abs: &Path, action: Action, reason: &str) {
    let (files, bytes) = measure(abs);
    entries.push(Entry {
        path,
        action,
        reason: reason.to_owned(),
        files,
        bytes,
    });
}

/// The path of a plan entry below `base` (`workspace/` or `files/`).
pub fn join(base: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(base.to_path_buf(), |p, part| p.join(part))
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
