//! Locate the `claude` CLI binary and verify it meets `MIN_CLI_VERSION`.
//!
//! We rely on `claude --version`, which prints a line of the form:
//!   `2.0.4 (Claude Code)`
//! The first whitespace-delimited token is the semver string we compare
//! against [`MIN_CLI_VERSION`].

use std::path::PathBuf;
use std::process::Command;

use super::types::{CliStatus, MIN_CLI_VERSION};

/// Locate the `claude` CLI binary.
///
/// Resolution order:
/// 1. `OPENHUMAN_CLAUDE_CLI` env override (tests / power users / a fixed path).
/// 2. `PATH` search.
/// 3. Well-known absolute install locations ([`well_known_candidates`]).
///
/// Step 3 exists because a macOS app launched from Finder/Dock inherits only
/// the stripped launchd `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), which never
/// contains the native installer's `~/.local/bin` — so a PATH-only lookup
/// reports the CLI "not installed" even though it is present. (Terminal
/// launches inherit the shell `PATH` and hit step 2, so this only bites GUI
/// launches.)
pub fn resolve_binary() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("OPENHUMAN_CLAUDE_CLI") {
        let p = PathBuf::from(explicit);
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(p) = which_on_path("claude") {
        return Some(p);
    }
    // PATH miss — fall back to well-known install locations. This is the
    // Finder/Dock-launch case where `~/.local/bin` is absent from `PATH`.
    let found = first_existing(&well_known_candidates());
    if let Some(p) = found.as_ref() {
        log::debug!(
            "[claude-code][version] `claude` not on PATH; resolved via well-known location {}",
            p.display()
        );
    }
    found
}

/// Absolute paths the `claude` CLI is commonly installed at, tried in order
/// when it is not found on `PATH`. Ordered by how the native installer and the
/// common package managers lay it down; the native installer's `~/.local/bin`
/// is first because that is the default and the one a stripped launchd `PATH`
/// omits.
fn well_known_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".local/bin/claude")); // native installer default
        out.push(home.join(".claude/local/claude")); // legacy local install
        out.push(home.join(".bun/bin/claude")); // bun global
        out.push(home.join(".npm-global/bin/claude")); // npm global (custom prefix)
        out.push(home.join("bin/claude"));
    }
    out.push(PathBuf::from("/opt/homebrew/bin/claude")); // Homebrew (Apple Silicon)
    out.push(PathBuf::from("/usr/local/bin/claude")); // Homebrew (Intel) / npm default
    out
}

/// First candidate that resolves to a file (follows symlinks — the native
/// installer's `~/.local/bin/claude` is a symlink into a versioned dir).
fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path_var) {
        if cfg!(windows) {
            for ext in &exts {
                let candidate = dir.join(format!("{name}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        } else {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Probe the `claude` CLI and return its status.
pub fn probe() -> CliStatus {
    let Some(path) = resolve_binary() else {
        log::debug!("[claude-code][version] no `claude` binary on PATH");
        return CliStatus::NotInstalled;
    };
    let path_str = path.display().to_string();

    let output = match Command::new(&path).arg("--version").output() {
        Ok(o) => o,
        Err(e) => {
            log::warn!("[claude-code][version] spawn failed path={path_str} err={e}");
            return CliStatus::Unusable {
                path: path_str,
                reason: format!("spawn failed: {e}"),
            };
        }
    };

    if !output.status.success() {
        return CliStatus::Unusable {
            path: path_str,
            reason: format!(
                "non-zero exit {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = match parse_version(&stdout) {
        Some(v) => v,
        None => {
            return CliStatus::Unusable {
                path: path_str,
                reason: format!("could not parse version from: {stdout:?}"),
            }
        }
    };

    if version_lt(&version, MIN_CLI_VERSION) {
        CliStatus::Outdated {
            version,
            min_required: MIN_CLI_VERSION.to_string(),
            path: path_str,
        }
    } else {
        CliStatus::Ok {
            version,
            path: path_str,
        }
    }
}

fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .next()
        .filter(|tok| tok.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|s| s.to_string())
}

/// Numeric semver compare. Returns true when `a < b`.
/// Pre-release suffixes (`-rc.1`) are stripped before comparison.
fn version_lt(a: &str, b: &str) -> bool {
    let pa = parts(a);
    let pb = parts(b);
    pa < pb
}

fn parts(v: &str) -> (u32, u32, u32) {
    let core = v.split('-').next().unwrap_or(v);
    let mut it = core.split('.').map(|s| s.parse::<u32>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_output() {
        assert_eq!(
            parse_version("2.0.4 (Claude Code)\n").as_deref(),
            Some("2.0.4")
        );
    }

    #[test]
    fn rejects_non_numeric_prefix() {
        assert_eq!(parse_version("claude version 2.0.4"), None);
    }

    #[test]
    fn version_compare() {
        assert!(version_lt("1.9.9", "2.0.0"));
        assert!(version_lt("2.0.0", "2.0.1"));
        assert!(!version_lt("2.0.0", "2.0.0"));
        assert!(!version_lt("2.1.0", "2.0.9"));
    }

    #[test]
    fn version_compare_strips_prerelease() {
        assert!(!version_lt("2.0.0-rc.1", "2.0.0"));
    }

    #[test]
    fn first_existing_picks_the_first_real_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing_a = dir.path().join("nope-a/claude");
        let missing_b = dir.path().join("nope-b/claude");
        let real = dir.path().join("claude");
        std::fs::write(&real, b"#!/bin/sh\n").expect("write fake binary");

        // Nothing present → None (the "CLI not installed" path).
        assert_eq!(first_existing(&[missing_a.clone(), missing_b.clone()]), None);
        // Skips the absent candidates and returns the first file that exists.
        assert_eq!(
            first_existing(&[missing_a, missing_b, real.clone()]),
            Some(real)
        );
    }

    #[test]
    fn well_known_candidates_lead_with_native_installer_path() {
        // The native installer default (`~/.local/bin/claude`) is the one a
        // stripped launchd PATH omits, so it must be the first fallback tried.
        let candidates = well_known_candidates();
        assert!(!candidates.is_empty());
        if let Some(home) = dirs::home_dir() {
            assert_eq!(candidates[0], home.join(".local/bin/claude"));
        }
    }
}
