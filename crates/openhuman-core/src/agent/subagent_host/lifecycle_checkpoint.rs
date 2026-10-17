//! Canonicalize subagent checkpoint paths before reserving a lifecycle.

use std::path::PathBuf;

pub(super) fn absolute_checkpoint_dir(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&path))
                .unwrap_or(path)
        }
    })
}
