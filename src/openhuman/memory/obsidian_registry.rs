//! Obsidian vault-*registration* detection for the memory content root.
//!
//! `obsidian://open?path=<abs>` only resolves against vaults already recorded
//! in Obsidian's `obsidian.json` registry — it can **not** register a new
//! vault, and a `.obsidian/` folder on disk is not enough. So before the
//! Memory tab fires that deep link, [`vault_registration_status`] answers
//! whether the content root (or an ancestor) is a vault Obsidian already
//! knows about. If it is not, the UI guides the user to add it once ("Open
//! folder as vault") instead of firing a link Obsidian rejects with *"Unable
//! to find a vault for the URL"*.
//!
//! Detection is **best-effort**: Obsidian can live in non-standard locations
//! (Flatpak, Snap, custom `$XDG_CONFIG_HOME`, portable). A negative result must
//! never block the user — the caller still offers "open anyway" + "reveal
//! folder" + a config-dir override that feeds back in here as `extra`.
//!
//! # Why this is OpenHuman's and not the engine's
//!
//! Same reasoning as [`crate::openhuman::memory::safety`] and
//! [`crate::openhuman::util::redact`] before it, and the same route: this is
//! **host desktop policy** that happened to live in the engine crate. Which
//! third-party note-taking app this host deep-links into, and where that app
//! keeps its config on each desktop OS, is a fact about the shipped desktop
//! product — not about how memory is stored. Its one caller
//! ([`crate::openhuman::memory::read_rpc::vault`]) is an RPC that reads the
//! filesystem and never touches a chunk.
//!
//! Widening `tinymemory-api` to carry it instead would be the wrong trade for
//! the reason `util::redact` records: the contract crate's manifest documents
//! that it stays dependency-light so a caller can depend on it and compile
//! almost nothing, and this policy costs `dirs` plus `serde_json`. Nor is it
//! contract vocabulary — nothing crosses the bus as a [`VaultRegistration`],
//! and "is my content root a registered Obsidian vault" is not a capability a
//! second memory driver would answer differently. A driver that stores chunks
//! in the cloud has no content root at all, and this probe would still be
//! asked of the same local directory.
//!
//! The engine keeps its own copy for its own callers. The two are independent
//! by design: neither reads the other's output, and both answer from the same
//! on-disk `obsidian.json`, which is Obsidian's file rather than a wire value
//! that has to agree across a boundary.
//!
//! # Ported verbatim
//!
//! Same candidate-path order, the same lexical (never canonicalising)
//! normalisation, the same component-boundary ancestor test, the same
//! empty-vault-path guard, and the same `config_found`-on-parse-failure
//! semantics — so the RPC above answers exactly what it always answered. The
//! engine's `types` submodule is inlined here because nothing outside the
//! module ever named it; every item keeps its name, so the two copies stay
//! diffable line for line. Only the log prefix changed, to this host's
//! `[memory]` grep convention.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::openhuman::util::redact::redact;

/// Outcome of a registration probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRegistration {
    /// `true` when some registered Obsidian vault's path equals or is an
    /// ancestor of the content root.
    pub registered: bool,
    /// `true` when at least one candidate `obsidian.json` was found/read (even
    /// if parsing it later fails — see the parse-error branch, which still
    /// counts the file as found). Lets the UI distinguish "Obsidian is set up,
    /// vault just not added yet" from "couldn't find Obsidian at all" (offer
    /// install vs. offer add-as-vault).
    pub config_found: bool,
}

/// Minimal shape of Obsidian's `obsidian.json`. We only need each vault's
/// `path`; `ts`/`open` and any future keys are ignored by `serde`.
#[derive(Debug, Deserialize)]
struct ObsidianConfig {
    #[serde(default)]
    vaults: HashMap<String, VaultEntry>,
}

/// One `vaults` entry of `obsidian.json`.
#[derive(Debug, Deserialize)]
struct VaultEntry {
    path: String,
}

/// Candidate `obsidian.json` locations, in priority order. `extra` (a
/// user-supplied override pointing at Obsidian's *config dir*) is checked
/// first so a power user can correct a non-standard install.
fn candidate_config_files(extra: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Some(dir) = extra {
        // Accept either the config dir itself or its parent (users often
        // can't tell whether the path should end in `obsidian/`).
        out.push(dir.join("obsidian.json"));
        out.push(dir.join("obsidian").join("obsidian.json"));
    }

    // Standard per-OS config dir: `~/.config` (Linux), `~/Library/Application
    // Support` (macOS), `%APPDATA%` (Windows).
    if let Some(cfg) = dirs::config_dir() {
        out.push(cfg.join("obsidian").join("obsidian.json"));
    }

    // Linux sandbox installs keep their own config tree. Harmless to probe on
    // other OSes — the paths simply won't exist.
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".var/app/md.obsidian.Obsidian/config/obsidian/obsidian.json")); // Flatpak
        out.push(home.join("snap/obsidian/current/.config/obsidian/obsidian.json"));
        // Snap
    }

    log::debug!(
        "[memory][obsidian_registry] candidate config files: n={}",
        out.len()
    );
    out
}

/// Best-effort: is `content_root` (or an ancestor) a registered Obsidian
/// vault? `extra_config_dir` optionally points at Obsidian's config dir for
/// non-standard installs. Never errors — probe failures report
/// `registered = false`.
#[must_use]
pub fn vault_registration_status(
    content_root: &Path,
    extra_config_dir: Option<&Path>,
) -> VaultRegistration {
    registration_in_files(content_root, &candidate_config_files(extra_config_dir))
}

/// Core of [`vault_registration_status`], split out so tests can supply an
/// explicit, isolated set of `obsidian.json` paths instead of depending on
/// whatever Obsidian config happens to exist on the host.
fn registration_in_files(content_root: &Path, files: &[PathBuf]) -> VaultRegistration {
    // Absolutize the content root against the current directory (lexically,
    // without touching the filesystem) so a relative content root compares
    // equal to the absolute vault paths Obsidian records in `obsidian.json`.
    // Without this, a relative root can never be a prefix of an absolute vault
    // path and an already-registered vault is always reported unregistered.
    let target = std::path::absolute(content_root)
        .map(|abs| lexically_normalize(&abs))
        .unwrap_or_else(|_| lexically_normalize(content_root));
    let mut config_found = false;

    for path in files {
        let body = match std::fs::read_to_string(path) {
            Ok(b) => b,
            Err(_) => continue, // missing/unreadable candidate — try the next.
        };
        config_found = true;

        let parsed: ObsidianConfig = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(err) => {
                // Redact the path — it embeds the user's home/username.
                log::warn!(
                    "[memory][obsidian_registry] parse {} failed: {err} — skipping",
                    redact(&path.display().to_string())
                );
                continue;
            }
        };

        for entry in parsed.vaults.values() {
            let vault = lexically_normalize(Path::new(&entry.path));
            // A malformed/empty vault path normalizes to "" and would otherwise
            // match every content root (empty ancestor ⊂ anything) — skip it.
            if vault.as_os_str().is_empty() {
                continue;
            }
            if is_ancestor_or_equal(&vault, &target) {
                log::debug!(
                    "[memory][obsidian_registry] content root is a registered vault \
                     (matched in {})",
                    redact(&path.display().to_string())
                );
                return VaultRegistration {
                    registered: true,
                    config_found: true,
                };
            }
        }
    }

    log::debug!(
        "[memory][obsidian_registry] content root NOT registered (config_found={config_found})"
    );
    VaultRegistration {
        registered: false,
        config_found,
    }
}

/// Strip trailing separators so `/a/b` and `/a/b/` compare equal. Lexical
/// only — we deliberately do not canonicalize: the vault path may be on an
/// unmounted volume or use a symlink, and canonicalize would error or rewrite
/// it. Both inputs come from trusted local sources, so a textual compare is
/// the safe, dependency-free choice.
fn lexically_normalize(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let trimmed = s.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        // Was a pure root like "/" — keep it.
        PathBuf::from(s.as_ref())
    } else {
        PathBuf::from(trimmed)
    }
}

/// `true` when `ancestor == descendant`, or `ancestor` is a path-prefix of
/// `descendant` on component boundaries (so `/a/b` contains `/a/b/c` but not
/// `/a/bc`). Case-sensitive — adequate for the Linux target; a false negative
/// on case-insensitive volumes only makes detection conservative (the caller
/// still offers "open anyway").
fn is_ancestor_or_equal(ancestor: &Path, descendant: &Path) -> bool {
    let a: Vec<_> = ancestor.components().collect();
    let d: Vec<_> = descendant.components().collect();
    // An empty ancestor must not match (it would otherwise be a prefix of
    // everything); also bail when the ancestor is longer than the descendant.
    if a.is_empty() || a.len() > d.len() {
        return false;
    }
    a.iter().zip(d.iter()).all(|(x, y)| x == y)
}

#[cfg(test)]
#[path = "obsidian_registry_tests.rs"]
mod tests;
