//! Canonicalize subagent checkpoint paths before reserving a lifecycle.

use std::path::PathBuf;

/// The in-flight reservation map is keyed by this path, so it must name a
/// directory identically before and after the lifecycle store creates it.
/// Canonicalizing the whole path only once it exists would resolve symlinked
/// parents (macOS `/var` -> `/private/var`) for a late caller but not an early
/// one, putting two callers of one task in different flights. Resolve the
/// deepest existing ancestor and re-append the not-yet-created tail instead.
pub(super) fn absolute_checkpoint_dir(path: PathBuf) -> PathBuf {
    let absolute = if path.is_absolute() {
        path
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(&path),
            Err(_) => path,
        }
    };
    let mut tail = Vec::new();
    let mut ancestor = absolute.as_path();
    loop {
        if let Ok(resolved) = std::fs::canonicalize(ancestor) {
            return tail.iter().rev().fold(resolved, |dir, name| dir.join(name));
        }
        match (ancestor.file_name(), ancestor.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_owned());
                ancestor = parent;
            }
            _ => return absolute,
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_checkpoint_tests.rs"]
mod tests;
