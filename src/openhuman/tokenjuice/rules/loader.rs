//! Three-layer rule loading: builtin → user → project.
//!
//! Port of `src/core/rules.ts` `loadRules()` logic.
//!
//! Layer order (lower priority → higher priority):
//! 1. builtin (embedded via `include_str!`)
//! 2. user (`~/.config/tokenjuice/rules/`)
//! 3. project (`<cwd>/.tokenjuice/rules/`)
//!
//! When two layers define the same `id`, the higher-priority layer wins
//! (project > user > builtin).  The `generic/fallback` rule is always sorted
//! last in the final list.

use super::{builtin::BUILTIN_RULE_JSONS, compiler::compile_rule};
use crate::openhuman::tokenjuice::types::{CompiledRule, JsonRule, RuleOrigin};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Options for `load_rules`.
#[derive(Debug, Default, Clone)]
pub struct LoadRuleOptions {
    /// Working directory for project-layer discovery.  Defaults to the process
    /// current directory.
    pub cwd: Option<PathBuf>,
    /// Override the user-layer directory (default: `~/.config/tokenjuice/rules`).
    pub user_rules_dir: Option<PathBuf>,
    /// Override the project-layer directory (default: `<cwd>/.tokenjuice/rules`).
    pub project_rules_dir: Option<PathBuf>,
    /// Skip user-layer rules.
    pub exclude_user: bool,
    /// Skip project-layer rules.
    pub exclude_project: bool,
}

// ---------------------------------------------------------------------------
// Layer path helpers
// ---------------------------------------------------------------------------

fn user_rules_root(custom: Option<&Path>) -> PathBuf {
    if let Some(p) = custom {
        return p.to_owned();
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("tokenjuice")
        .join("rules")
}

fn project_rules_root(cwd: Option<&Path>, custom: Option<&Path>) -> PathBuf {
    if let Some(p) = custom {
        return p.to_owned();
    }
    cwd.unwrap_or_else(|| Path::new("."))
        .join(".tokenjuice")
        .join("rules")
}

// ---------------------------------------------------------------------------
// Builtin layer
// ---------------------------------------------------------------------------

fn load_builtin_descriptors() -> Vec<(RuleOrigin, String, JsonRule)> {
    BUILTIN_RULE_JSONS
        .iter()
        .filter_map(|(id, json)| match serde_json::from_str::<JsonRule>(json) {
            Ok(rule) => {
                log::debug!("[tokenjuice] loaded builtin rule '{}'", id);
                Some((RuleOrigin::Builtin, format!("builtin:{}", id), rule))
            }
            Err(err) => {
                log::debug!(
                    "[tokenjuice] failed to parse builtin rule '{}': {}",
                    id,
                    err
                );
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Disk layer
// ---------------------------------------------------------------------------

/// Recursively walk `root` and return all `.json` files that are not
/// `.schema.json` or `.fixture.json`.
fn list_rule_files(root: &Path) -> Vec<PathBuf> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk_dir(root, &mut out);
    out.sort();
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            log::debug!("[tokenjuice] read_dir failed at {}: {}", dir.display(), err);
            return;
        }
    };
    let mut names: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    names.sort_by_key(|e| e.file_name());

    for entry in names {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(err) => {
                log::debug!(
                    "[tokenjuice] file_type failed at {}: {}",
                    path.display(),
                    err
                );
                continue;
            }
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk_dir(&path, out);
        } else if ft.is_file() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".json")
                && !name_str.ends_with(".schema.json")
                && !name_str.ends_with(".fixture.json")
            {
                out.push(path);
            }
        }
    }
}

fn load_disk_descriptors(root: &Path, source: RuleOrigin) -> Vec<(RuleOrigin, String, JsonRule)> {
    let files = list_rule_files(root);
    files
        .into_iter()
        .filter_map(|path| {
            let json = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(err) => {
                    log::debug!(
                        "[tokenjuice] read_to_string failed for {:?} rule at {}: {}",
                        source,
                        path.display(),
                        err
                    );
                    return None;
                }
            };
            match serde_json::from_str::<JsonRule>(&json) {
                Ok(rule) => {
                    log::debug!(
                        "[tokenjuice] loaded {:?} rule '{}' from {}",
                        source,
                        rule.id,
                        path.display()
                    );
                    Some((source.clone(), path.display().to_string(), rule))
                }
                Err(err) => {
                    log::debug!(
                        "[tokenjuice] failed to parse {:?} rule at {}: {}",
                        source,
                        path.display(),
                        err
                    );
                    None
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Overlay & sort
// ---------------------------------------------------------------------------

/// Merge descriptors by `rule.id`: later entries win (project > user > builtin).
fn overlay_and_sort(descriptors: Vec<(RuleOrigin, String, JsonRule)>) -> Vec<CompiledRule> {
    // Use an IndexMap-like approach via a Vec to preserve last-write semantics
    // while keeping insertion order (needed for stable sort).
    let mut by_id: std::collections::HashMap<String, (RuleOrigin, String, JsonRule)> =
        std::collections::HashMap::new();

    for (source, path, rule) in descriptors {
        by_id.insert(rule.id.clone(), (source, path, rule));
    }

    let mut compiled: Vec<CompiledRule> = by_id
        .into_values()
        .map(|(source, path, rule)| compile_rule(rule, source, path))
        .collect();

    // Sort alphabetically, `generic/fallback` last
    compiled.sort_by(|a, b| {
        let a_fb = a.rule.id == "generic/fallback";
        let b_fb = b.rule.id == "generic/fallback";
        match (a_fb, b_fb) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => a.rule.id.cmp(&b.rule.id),
        }
    });

    log::debug!(
        "[tokenjuice] overlay resolved {} rules (fallback last)",
        compiled.len()
    );

    compiled
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load and compile all rules from the three-layer overlay.
///
/// Layers are resolved in priority order (builtin < user < project) so that
/// a project rule with the same `id` overrides a builtin rule.
pub fn load_rules(opts: &LoadRuleOptions) -> Vec<CompiledRule> {
    let mut descriptors: Vec<(RuleOrigin, String, JsonRule)> = Vec::new();

    // 1. Builtin (lowest priority)
    descriptors.extend(load_builtin_descriptors());

    // 2. User layer
    if !opts.exclude_user {
        let user_root = user_rules_root(opts.user_rules_dir.as_deref());
        log::debug!(
            "[tokenjuice] loading user rules from {}",
            user_root.display()
        );
        descriptors.extend(load_disk_descriptors(&user_root, RuleOrigin::User));
    }

    // 3. Project layer (highest priority)
    if !opts.exclude_project {
        let project_root =
            project_rules_root(opts.cwd.as_deref(), opts.project_rules_dir.as_deref());
        log::debug!(
            "[tokenjuice] loading project rules from {}",
            project_root.display()
        );
        descriptors.extend(load_disk_descriptors(&project_root, RuleOrigin::Project));
    }

    overlay_and_sort(descriptors)
}

/// Load only the builtin rules (no disk I/O).
pub fn load_builtin_rules() -> Vec<CompiledRule> {
    load_rules(&LoadRuleOptions {
        exclude_user: true,
        exclude_project: true,
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "loader_tests.rs"]
mod tests;
