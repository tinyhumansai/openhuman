//! Making a caller's skill bundles visible to the agent.
//!
//! # Why this copies rather than symlinks
//!
//! Skill discovery scans a fixed set of roots derived from the workspace and
//! `$HOME`; there is no parameter for an extra one and no in-memory
//! registration — a `Workflow` only exists by parsing files. The one root a
//! harness controls without a trust marker is `<workspace>/skills`, so that is
//! where a caller's bundles have to end up.
//!
//! Linking them there does not work, and the reason is deliberate rather than
//! incidental. `scan_root_inner` uses `file_type()` instead of `is_dir()`
//! specifically so a symlinked child cannot be loaded as a skill, and
//! `load_skill_dir` requires a real non-symlink regular file for the manifest
//! because `exists()` follows links and would otherwise ingest an arbitrary
//! file's contents into the prompt flow. Both are security controls on a root
//! scanned *without* a trust marker. Copying respects them; a symlink would be
//! silently skipped, which is the worst outcome — skills that appear configured
//! and are simply absent from the turn.
//!
//! # Why an inherited workspace refuses
//!
//! Under [`Workspace::Inherit`](super::Workspace) the skills root is the
//! operator's own. Copying into it would leave bundles behind after the process
//! exits, shadowing or colliding with skills the operator installed. A library
//! call should not do that, so it is an error rather than a surprise.

use std::path::{Path, PathBuf};

use super::error::HarnessError;

/// The manifest filenames a skill bundle can declare itself with, newest first.
const MANIFESTS: [&str; 3] = ["WORKFLOW.md", "SKILL.md", "skill.json"];

/// Copy every skill bundle in `source` into `<workspace>/skills`.
///
/// A "bundle" is an immediate subdirectory carrying one of [`MANIFESTS`]. If
/// `source` itself carries one, it is treated as a single bundle and copied
/// under its own directory name — so both `skills_dir("./skills")` and
/// `skills_dir("./skills/my-skill")` do what the caller plainly meant.
pub(super) fn install(source: &Path, workspace_dir: &Path) -> Result<usize, HarnessError> {
    if !source.is_dir() {
        return Err(HarnessError::Invalid(format!(
            "skills_dir {} is not a directory",
            source.display()
        )));
    }

    let dest_root = workspace_dir.join("skills");
    std::fs::create_dir_all(&dest_root).map_err(|source| HarnessError::Workspace {
        what: "create the workspace skills directory",
        source,
    })?;

    let bundles = if is_bundle(source) {
        vec![source.to_path_buf()]
    } else {
        collect_bundles(source)?
    };

    if bundles.is_empty() {
        // Not an error: an empty skills directory is a legitimate state (a
        // caller wiring the flag up before authoring anything). Log it, because
        // the alternative failure mode — silently running with no skills when
        // the caller expected some — is the one worth making visible.
        log::warn!(
            "[embed][harness] skills_dir {} contains no skill bundles \
             (a bundle is a directory holding one of {MANIFESTS:?})",
            source.display()
        );
        return Ok(0);
    }

    for bundle in &bundles {
        let name = bundle
            .file_name()
            .ok_or_else(|| HarnessError::Invalid(format!("unnamed skill bundle {bundle:?}")))?;
        copy_tree(bundle, &dest_root.join(name))?;
    }

    log::debug!(
        "[embed][harness] installed {} skill bundle(s) from {} into {}",
        bundles.len(),
        source.display(),
        dest_root.display()
    );
    Ok(bundles.len())
}

/// Whether `dir` declares itself a skill bundle.
fn is_bundle(dir: &Path) -> bool {
    MANIFESTS.iter().any(|manifest| {
        // `is_file()` follows symlinks; discovery requires a real regular
        // file, so a symlinked manifest must not make this a bundle. `copy_tree`
        // skips symlinks, so counting a symlinked manifest as a bundle would
        // install a directory with no manifest — a bundle that never reaches a
        // turn, the exact silent-absence this module guards against.
        std::fs::symlink_metadata(dir.join(manifest))
            .map(|meta| meta.file_type().is_file())
            .unwrap_or(false)
    })
}

fn collect_bundles(root: &Path) -> Result<Vec<PathBuf>, HarnessError> {
    let entries = std::fs::read_dir(root).map_err(|source| HarnessError::Workspace {
        what: "read the skills directory",
        source,
    })?;

    let mut bundles: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            // Mirror discovery's own rule: a symlinked child is not a bundle.
            // Copying one would smuggle in exactly what discovery refuses.
            entry
                .file_type()
                .map(|ty| ty.is_dir() && !ty.is_symlink())
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .filter(|path| is_bundle(path))
        .collect();

    // `read_dir` order is unspecified; sort so a copy is reproducible.
    bundles.sort();
    Ok(bundles)
}

/// Recursively copy `from` to `to`, skipping symlinks.
///
/// Symlinks are skipped rather than followed for the same reason discovery
/// rejects them: a link inside a bundle can point anywhere, and copying its
/// target would pull outside content into a directory the agent reads.
fn copy_tree(from: &Path, to: &Path) -> Result<(), HarnessError> {
    std::fs::create_dir_all(to).map_err(|source| HarnessError::Workspace {
        what: "create a skill bundle directory",
        source,
    })?;

    let entries = std::fs::read_dir(from).map_err(|source| HarnessError::Workspace {
        what: "read a skill bundle directory",
        source,
    })?;

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            log::debug!(
                "[embed][harness] skipping symlink {} inside a skill bundle",
                entry.path().display()
            );
            continue;
        }
        let target = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|source| HarnessError::Workspace {
                what: "copy a skill bundle file",
                source,
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
