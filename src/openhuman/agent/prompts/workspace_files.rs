use crate::openhuman::agent::prompts::types::BOOTSTRAP_MAX_CHARS;
use std::fmt::Write;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Ensure the workspace file is up-to-date with the compiled-in default.
///
/// On first install the file doesn't exist → write it. On subsequent runs
/// we store a hash of the compiled-in content in a sidecar file
/// (`.{filename}.builtin-hash`). If the hash changes (code was updated),
/// the disk file is overwritten so prompt improvements ship automatically.
/// User edits between code releases are preserved — we only overwrite when
/// the built-in default itself changes.
pub(crate) fn sync_workspace_file(workspace_dir: &Path, filename: &str) {
    let default_content = default_workspace_file_content(filename);
    if default_content.is_empty() {
        return;
    }

    let path = workspace_dir.join(filename);
    let hash_path = workspace_dir.join(format!(".{filename}.builtin-hash"));

    // Compute a simple hash of the current compiled-in content.
    let current_hash = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        default_content.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    };

    // Read the last-written hash (if any).
    let stored_hash = std::fs::read_to_string(&hash_path).unwrap_or_default();
    let stored_hash = stored_hash.trim();

    if stored_hash == current_hash && path.exists() {
        // Built-in hasn't changed and file exists — nothing to do.
        return;
    }

    // Decide whether to overwrite the existing file. Two safe cases:
    //   1. File doesn't exist yet — first install, write the default.
    //   2. File exists AND its current hash matches the stored builtin
    //      hash — the user hasn't edited it since we last wrote it, so
    //      it's safe to ship the new default.
    // Otherwise the file has been hand-edited between releases; leave
    // the user's version in place and just update the stored hash so we
    // stop re-comparing against the old default on every boot.
    let file_exists = path.exists();
    let user_unmodified = if file_exists {
        match std::fs::read_to_string(&path) {
            Ok(disk) => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                disk.hash(&mut hasher);
                let disk_hash = format!("{:016x}", hasher.finish());
                disk_hash == stored_hash
            }
            Err(_) => false,
        }
    } else {
        false
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if !file_exists || user_unmodified {
        if let Err(e) = std::fs::write(&path, default_content) {
            log::warn!("[agent:prompt] failed to write workspace file {filename}: {e}");
            return;
        }
        log::info!("[agent:prompt] updated workspace file {filename} (builtin content changed)");
    } else {
        log::info!(
            "[agent:prompt] keeping user-edited workspace file {filename} (builtin changed but disk contents diverge)"
        );
    }
    let _ = std::fs::write(&hash_path, &current_hash);
}

/// Inject `filename` from `workspace_dir` into `prompt`, truncated to
/// [`BOOTSTRAP_MAX_CHARS`]. Thin wrapper around
/// [`inject_workspace_file_capped`] for bootstrap-class files
/// (`SOUL.md`, `IDENTITY.md`, `HEARTBEAT.md`).
pub(crate) fn inject_workspace_file(prompt: &mut String, workspace_dir: &Path, filename: &str) {
    inject_workspace_file_capped(prompt, workspace_dir, filename, BOOTSTRAP_MAX_CHARS);
}

/// Inject `content` into `prompt` under a header matching
/// [`inject_workspace_file_capped`]'s format — so a swap from the
/// file-based loader to a curated-memory snapshot is byte-compatible
/// for the output header and truncation semantics.
///
/// Empty/whitespace content is silently skipped, mirroring the file
/// loader's "no noisy placeholder" behaviour.
pub(crate) fn inject_snapshot_content(
    prompt: &mut String,
    label: &str,
    content: &str,
    max_chars: usize,
) {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }
    let _ = writeln!(prompt, "### {label}\n");
    let truncated = if trimmed.chars().count() > max_chars {
        trimmed
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| &trimmed[..idx])
            .unwrap_or(trimmed)
    } else {
        trimmed
    };
    prompt.push_str(truncated);
    if truncated.len() < trimmed.len() {
        let _ = writeln!(
            prompt,
            "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
        );
    } else {
        prompt.push_str("\n\n");
    }
}

/// Inject `filename` into `prompt` with an explicit character budget.
///
/// Used directly by callers that want a tighter cap than
/// [`BOOTSTRAP_MAX_CHARS`] — notably `PROFILE.md` and `MEMORY.md` which
/// are user-specific, potentially growing, and do not warrant a full
/// 20K-char budget (see [`USER_FILE_MAX_CHARS`]).
///
/// Missing / empty files are silently skipped so callers can inject
/// optional files unconditionally without emitting a noisy placeholder.
///
/// **KV-cache contract:** the output is a pure function of `filename`,
/// file bytes at call time, and `max_chars`. Callers must invoke this
/// once per session — re-reading mid-session breaks the inference
/// backend's automatic prefix cache. See the byte-stability note on
/// [`render_subagent_system_prompt`].
pub(crate) fn inject_workspace_file_capped(
    prompt: &mut String,
    workspace_dir: &Path,
    filename: &str,
    max_chars: usize,
) {
    let path = workspace_dir.join(filename);

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let _ = writeln!(prompt, "### {filename}\n");
            let truncated = if trimmed.chars().count() > max_chars {
                trimmed
                    .char_indices()
                    .nth(max_chars)
                    .map(|(idx, _)| &trimmed[..idx])
                    .unwrap_or(trimmed)
            } else {
                trimmed
            };
            prompt.push_str(truncated);
            if truncated.len() < trimmed.len() {
                let _ = writeln!(
                    prompt,
                    "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
                );
            } else {
                prompt.push_str("\n\n");
            }
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => {
                // Keep prompt focused: missing optional identity/bootstrap files should not
                // add noisy placeholders that dilute tool-calling instructions.
            }
            _ => {
                log::debug!("[prompt] failed to read {}: {e}", path.display());
            }
        },
    }
}

pub(crate) fn default_workspace_file_content(filename: &str) -> &'static str {
    // The bundled identity files live at `src/openhuman/agent/prompts/`
    // (owned by the `agent/` tree because they describe agent identity).
    // This module is under `src/openhuman/context/`, so the relative path
    // walks up one level and back into `agent/prompts/`.
    match filename {
        "SOUL.md" => include_str!("SOUL.md"),
        "IDENTITY.md" => include_str!("IDENTITY.md"),
        "HEARTBEAT.md" => {
            "# Periodic Tasks\n\n# Add tasks below (one per line, starting with `- `)\n"
        }
        _ => "",
    }
}
