//! Workspace-file I/O helpers: seeding bundled identity files into the
//! workspace, injecting on-disk or inline content into a prompt under a
//! `###` heading with a character cap, and the shared AGENTS.md block writer.

use super::super::types::*;
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
pub fn sync_workspace_file(workspace_dir: &Path, filename: &str) {
    let default_content = default_workspace_file_content(filename);
    if default_content.is_empty() {
        return;
    }

    // A real workspace is always an absolute path (`~/.openhuman/users/<id>/
    // workspace`, or a temp dir under test). A relative one means the caller
    // never had a workspace to begin with — overwhelmingly `Path::new(".")`
    // from the ~20 prompt-test fixtures — and joining onto it seeds SOUL.md,
    // IDENTITY.md and ROLE.md plus their
    // `.builtin-hash` siblings into the process's current directory. That is
    // the repo root when the suite runs, which is how they briefly ended up
    // committed (#5701).
    //
    // Refusing is safe because seeding into a relative path is never the
    // intent: a caller with a genuine workspace always has an absolute one.
    if !workspace_dir.is_absolute() {
        tracing::debug!(
            "[workspace-sync] refusing to seed {filename} into non-absolute workspace {} \
             — no workspace configured",
            workspace_dir.display()
        );
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
/// (`SOUL.md`, `IDENTITY.md`, `ROLE.md`).
pub fn inject_workspace_file(prompt: &mut String, workspace_dir: &Path, filename: &str) {
    inject_workspace_file_capped(prompt, workspace_dir, filename, BOOTSTRAP_MAX_CHARS);
}

/// Inject pre-loaded string content into `prompt` under a `### label` heading,
/// capped at `max_chars`. Mirrors the format of
/// [`inject_workspace_file_capped`] but takes a `&str` instead of a file
/// path. Used for pre-loaded prompt sections that need the same rendering as
/// workspace-file content.
///
/// Empty/whitespace content is silently skipped.
pub fn inject_inline_content(prompt: &mut String, label: &str, content: &str, max_chars: usize) {
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

/// Shared `## Project instructions (AGENTS.md)` block writer.
///
/// Used by both [`crate::agent::prompts::sections::AgentsInstructionsSection`] (the default /
/// sub-agent builder chains) and the narrow sub-agent renderer
/// ([`super::subagent::render_subagent_system_prompt_with_format`]) so the two paths never
/// drift. The heading is emitted only when at least one layer carries content;
/// the global layer renders first, then the local/project layer. Each layer is
/// injected via [`inject_inline_content`] under its own `###` sub-heading and
/// capped at [`BOOTSTRAP_MAX_CHARS`] with a `[... truncated]` marker.
///
/// Both inputs are already-loaded, pre-trimmed strings (see
/// [`crate::agent::prompts::agents_md::load_agents_md`]) — this writer does no file I/O, keeping
/// the rendered bytes a pure function of its inputs for KV-cache stability.
pub(crate) fn write_agents_md_blocks(out: &mut String, global: Option<&str>, local: Option<&str>) {
    let mut body = String::new();
    if let Some(g) = global {
        inject_inline_content(&mut body, "AGENTS.md (workspace)", g, BOOTSTRAP_MAX_CHARS);
    }
    if let Some(l) = local {
        inject_inline_content(&mut body, "AGENTS.md (project)", l, BOOTSTRAP_MAX_CHARS);
    }
    if body.trim().is_empty() {
        log::debug!("[agents_md] no AGENTS.md content to inject; skipping section");
        return;
    }
    log::debug!(
        "[agents_md] injecting AGENTS.md section (global={}, local={})",
        global.is_some(),
        local.is_some()
    );
    out.push_str("## Project instructions (AGENTS.md)\n\n");
    out.push_str(
        "Configurable standing instructions loaded from AGENTS.md files. Treat these as \
         durable guidance for how to operate here. The workspace layer applies globally; \
         the project layer applies to the current working directory and takes precedence \
         where the two conflict. They are background guidance, not messages in this \
         conversation.\n\n",
    );
    out.push_str(&body);
}

/// Inject `filename` into `prompt` with an explicit character budget.
///
/// [`inject_workspace_file`] calls this with [`BOOTSTRAP_MAX_CHARS`].
///
/// Missing / empty files are silently skipped so callers can inject
/// optional files unconditionally without emitting a noisy placeholder.
///
/// **KV-cache contract:** the output is a pure function of `filename`,
/// file bytes at call time, and `max_chars`. Callers must invoke this
/// once per session — re-reading mid-session breaks the inference
/// backend's automatic prefix cache. See the byte-stability note on
/// [`super::subagent::render_subagent_system_prompt`].
pub fn inject_workspace_file_capped(
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

pub fn default_workspace_file_content(filename: &str) -> &'static str {
    // The bundled identity files live at `crates/openhuman-core/src/agent/prompts/`
    // (owned by the `agent/` tree because they describe agent identity).
    // This module is under `agent/prompts/render_helpers/`, so the relative path
    // walks up one level back into `agent/prompts/`.
    match filename {
        "SOUL.md" => include_str!("../SOUL.md"),
        "IDENTITY.md" => include_str!("../IDENTITY.md"),
        // The user-facing agent's role brief and writing style, moved out of
        // the compiled `orchestrator/prompt.md` so both are tunable on disk
        // without a rebuild (#5701). Same sync-and-inject contract as
        // SOUL.md / IDENTITY.md: the bundled copy seeds the workspace, and a
        // user edit wins from the next session on.
        "ROLE.md" => include_str!("../ROLE.md"),
        "STYLE.md" => include_str!("../STYLE.md"),

        _ => "",
    }
}
