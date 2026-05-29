//! Obsidian vault-*registration* detection.
//!
//! Sibling to [`super::obsidian`] (which writes the `.obsidian/` *defaults*
//! into the content root). This module answers a different question: is the
//! content root actually a vault Obsidian knows about?
//!
//! `obsidian://open?path=<abs>` only resolves against vaults already recorded
//! in Obsidian's `obsidian.json` registry — it can **not** register a new
//! vault, and a `.obsidian/` folder on disk is not enough. So before the
//! Memory tab fires that deep link we check whether the content root (or an
//! ancestor) is a registered vault. If it isn't, the UI guides the user to add
//! it once ("Open folder as vault") instead of firing a link Obsidian rejects
//! with *"Unable to find a vault for the URL"*.
//!
//! Detection is **best-effort**: Obsidian can live in non-standard locations
//! (Flatpak, Snap, custom `$XDG_CONFIG_HOME`, portable). A negative result must
//! never block the user — the caller still offers "open anyway" + "reveal
//! folder" + a config-dir override that feeds back in here as `extra`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

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
    vaults: std::collections::HashMap<String, VaultEntry>,
}

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

    out
}

/// Best-effort: is `content_root` (or an ancestor) a registered Obsidian
/// vault? `extra_config_dir` optionally points at Obsidian's config dir for
/// non-standard installs. Never errors — probe failures report
/// `registered = false`.
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
    let target = lexically_normalize(content_root);
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
                    "[content_store::obsidian_registry] parse {} failed: {err} — skipping",
                    crate::openhuman::memory::util::redact::redact(&path.display().to_string())
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
                    "[content_store::obsidian_registry] content root is a registered vault \
                     (matched in {})",
                    crate::openhuman::memory::util::redact::redact(&path.display().to_string())
                );
                return VaultRegistration {
                    registered: true,
                    config_found: true,
                };
            }
        }
    }

    log::debug!(
        "[content_store::obsidian_registry] content root NOT registered (config_found={})",
        config_found
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
mod tests {
    use super::*;
    use std::io::Write;

    /// Write an `obsidian.json` containing `vault_paths` and return its path.
    fn write_config(dir: &Path, vault_paths: &[&str]) -> PathBuf {
        let entries: Vec<String> = vault_paths
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    "\"id{i}\": {{ \"path\": {}, \"ts\": 1700000000000, \"open\": true }}",
                    serde_json::to_string(p).unwrap()
                )
            })
            .collect();
        let body = format!("{{ \"vaults\": {{ {} }} }}", entries.join(", "));
        let path = dir.join("obsidian.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn exact_match_is_registered() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("memory_tree/content");
        let cfg = write_config(tmp.path(), &[root.to_str().unwrap()]);
        let got = registration_in_files(&root, &[cfg]);
        assert_eq!(
            got,
            VaultRegistration {
                registered: true,
                config_found: true
            }
        );
    }

    #[test]
    fn ancestor_vault_is_registered() {
        // A vault rooted at the parent still "contains" the content root.
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("workspace");
        let root = parent.join("memory_tree/content");
        let cfg = write_config(tmp.path(), &[parent.to_str().unwrap()]);
        assert!(registration_in_files(&root, &[cfg]).registered);
    }

    #[test]
    fn trailing_slash_does_not_matter() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("memory_tree/content");
        let with_slash = format!("{}/", root.to_str().unwrap());
        let cfg = write_config(tmp.path(), &[&with_slash]);
        assert!(registration_in_files(&root, &[cfg]).registered);
    }

    #[test]
    fn unrelated_vault_is_not_registered_but_config_found() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("memory_tree/content");
        let cfg = write_config(tmp.path(), &["/some/other/vault"]);
        let got = registration_in_files(&root, &[cfg]);
        assert_eq!(
            got,
            VaultRegistration {
                registered: false,
                config_found: true
            }
        );
    }

    #[test]
    fn empty_vault_path_does_not_match_every_root() {
        // Regression: a malformed entry with an empty `path` must not
        // normalize to "" and match every content root as an ancestor.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("memory_tree/content");
        let cfg = write_config(tmp.path(), &[""]);
        let got = registration_in_files(&root, &[cfg]);
        assert_eq!(
            got,
            VaultRegistration {
                registered: false,
                config_found: true
            }
        );
    }

    #[test]
    fn sibling_prefix_is_not_a_false_match() {
        // `/a/b/content` must NOT match a vault at `/a/b/content-archive`.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("content");
        let decoy = format!("{}-archive", root.to_str().unwrap());
        let cfg = write_config(tmp.path(), &[&decoy]);
        assert!(!registration_in_files(&root, &[cfg]).registered);
    }

    #[test]
    fn missing_config_reports_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("memory_tree/content");
        let missing = tmp.path().join("does-not-exist.json");
        let got = registration_in_files(&root, &[missing]);
        assert_eq!(
            got,
            VaultRegistration {
                registered: false,
                config_found: false
            }
        );
    }

    #[test]
    fn malformed_config_is_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("memory_tree/content");
        let bad = tmp.path().join("obsidian.json");
        std::fs::write(&bad, b"{ this is not json ").unwrap();
        // config_found is true (we read it) but parse fails → not registered.
        let got = registration_in_files(&root, &[bad]);
        assert_eq!(
            got,
            VaultRegistration {
                registered: false,
                config_found: true
            }
        );
    }

    #[test]
    fn second_candidate_wins_when_first_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("memory_tree/content");
        let missing = tmp.path().join("nope.json");
        let real = write_config(tmp.path(), &[root.to_str().unwrap()]);
        assert!(registration_in_files(&root, &[missing, real]).registered);
    }
}
