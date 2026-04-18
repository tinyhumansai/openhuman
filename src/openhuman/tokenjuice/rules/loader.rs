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
mod tests {
    use super::*;

    #[test]
    fn builtin_rules_load_successfully() {
        let rules = load_builtin_rules();
        assert!(!rules.is_empty(), "at least one built-in rule expected");
        let ids: Vec<&str> = rules.iter().map(|r| r.rule.id.as_str()).collect();
        assert!(
            ids.contains(&"generic/fallback"),
            "generic/fallback must be present"
        );
    }

    #[test]
    fn fallback_rule_is_last() {
        let rules = load_builtin_rules();
        let last = rules.last().expect("non-empty list");
        assert_eq!(last.rule.id, "generic/fallback");
    }

    #[test]
    fn project_layer_overrides_builtin() {
        // Write a temporary project rules dir with a modified fallback rule
        let dir = tempfile::tempdir().expect("tempdir");
        let override_json = r#"{
            "id": "generic/fallback",
            "family": "override-family",
            "description": "overridden",
            "match": {}
        }"#;
        std::fs::write(dir.path().join("fallback.json"), override_json).unwrap();

        let opts = LoadRuleOptions {
            project_rules_dir: Some(dir.path().to_owned()),
            exclude_user: true,
            ..Default::default()
        };
        let rules = load_rules(&opts);
        let fb = rules
            .iter()
            .find(|r| r.rule.id == "generic/fallback")
            .expect("fallback rule");
        assert_eq!(fb.rule.family, "override-family");
        assert_eq!(fb.source, RuleOrigin::Project);
    }

    #[test]
    fn rules_sorted_alphabetically_fallback_last() {
        let rules = load_builtin_rules();
        let non_fb: Vec<&str> = rules
            .iter()
            .filter(|r| r.rule.id != "generic/fallback")
            .map(|r| r.rule.id.as_str())
            .collect();
        let mut sorted = non_fb.clone();
        sorted.sort();
        assert_eq!(non_fb, sorted, "rules should be alphabetically sorted");
    }

    // --- load_rules with disk layers ---

    #[test]
    fn user_layer_overrides_builtin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let override_json = r#"{
            "id": "git/status",
            "family": "user-overridden",
            "description": "user override",
            "match": {}
        }"#;
        std::fs::write(dir.path().join("git_status.json"), override_json).unwrap();

        let opts = LoadRuleOptions {
            user_rules_dir: Some(dir.path().to_owned()),
            exclude_project: true,
            ..Default::default()
        };
        let rules = load_rules(&opts);
        let gs = rules
            .iter()
            .find(|r| r.rule.id == "git/status")
            .expect("git/status rule");
        assert_eq!(gs.rule.family, "user-overridden");
        assert_eq!(gs.source, RuleOrigin::User);
    }

    #[test]
    fn invalid_json_files_are_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Write an invalid JSON file
        std::fs::write(dir.path().join("bad.json"), "{ this is not valid json }").unwrap();
        // Write a valid rule
        let valid_json = r#"{
            "id": "test/valid",
            "family": "test",
            "match": {}
        }"#;
        std::fs::write(dir.path().join("valid.json"), valid_json).unwrap();

        let opts = LoadRuleOptions {
            project_rules_dir: Some(dir.path().to_owned()),
            exclude_user: true,
            ..Default::default()
        };
        let rules = load_rules(&opts);
        // Valid rule should be loaded, invalid should be silently skipped
        assert!(rules.iter().any(|r| r.rule.id == "test/valid"));
    }

    #[test]
    fn schema_and_fixture_json_files_are_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        // These should be ignored by list_rule_files
        std::fs::write(
            dir.path().join("rules.schema.json"),
            r#"{"id":"should-skip","family":"skip","match":{}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("example.fixture.json"),
            r#"{"id":"should-skip2","family":"skip","match":{}}"#,
        )
        .unwrap();
        // A normal rule that should be loaded
        std::fs::write(
            dir.path().join("normal.json"),
            r#"{"id":"test/normal","family":"test","match":{}}"#,
        )
        .unwrap();

        let opts = LoadRuleOptions {
            project_rules_dir: Some(dir.path().to_owned()),
            exclude_user: true,
            ..Default::default()
        };
        let rules = load_rules(&opts);
        // schema/fixture files should not be loaded
        assert!(!rules.iter().any(|r| r.rule.id == "should-skip"));
        assert!(!rules.iter().any(|r| r.rule.id == "should-skip2"));
        // Normal rule should be there
        assert!(rules.iter().any(|r| r.rule.id == "test/normal"));
    }

    #[test]
    fn non_existent_dir_loads_only_builtins() {
        let opts = LoadRuleOptions {
            user_rules_dir: Some(std::path::PathBuf::from(
                "/nonexistent/path/that/does/not/exist",
            )),
            project_rules_dir: Some(std::path::PathBuf::from("/another/nonexistent/path/rules")),
            ..Default::default()
        };
        let rules = load_rules(&opts);
        // Should still have builtins
        assert!(rules.iter().any(|r| r.rule.id == "generic/fallback"));
        assert!(!rules.is_empty());
    }

    #[test]
    fn exclude_user_skips_user_layer() {
        let user_dir = tempfile::tempdir().expect("tempdir");
        let override_json = r#"{"id":"git/status","family":"should-not-see","match":{}}"#;
        std::fs::write(user_dir.path().join("override.json"), override_json).unwrap();

        let opts = LoadRuleOptions {
            user_rules_dir: Some(user_dir.path().to_owned()),
            exclude_user: true,
            exclude_project: true,
            ..Default::default()
        };
        let rules = load_rules(&opts);
        // user override should NOT be present — original builtin should remain
        let gs = rules
            .iter()
            .find(|r| r.rule.id == "git/status")
            .expect("git/status");
        assert_ne!(gs.rule.family, "should-not-see");
        assert_eq!(gs.source, RuleOrigin::Builtin);
    }

    #[test]
    fn project_layer_wins_over_user_layer() {
        let user_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        std::fs::write(
            user_dir.path().join("rule.json"),
            r#"{"id":"git/status","family":"user-family","match":{}}"#,
        )
        .unwrap();
        std::fs::write(
            project_dir.path().join("rule.json"),
            r#"{"id":"git/status","family":"project-family","match":{}}"#,
        )
        .unwrap();

        let opts = LoadRuleOptions {
            user_rules_dir: Some(user_dir.path().to_owned()),
            project_rules_dir: Some(project_dir.path().to_owned()),
            ..Default::default()
        };
        let rules = load_rules(&opts);
        let gs = rules
            .iter()
            .find(|r| r.rule.id == "git/status")
            .expect("git/status");
        // Project wins over user
        assert_eq!(gs.rule.family, "project-family");
        assert_eq!(gs.source, RuleOrigin::Project);
    }

    #[test]
    fn subdirectory_rules_are_discovered() {
        let dir = tempfile::tempdir().expect("tempdir");
        let subdir = dir.path().join("git");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(
            subdir.join("my_rule.json"),
            r#"{"id":"test/subdir-rule","family":"test","match":{}}"#,
        )
        .unwrap();

        let opts = LoadRuleOptions {
            project_rules_dir: Some(dir.path().to_owned()),
            exclude_user: true,
            ..Default::default()
        };
        let rules = load_rules(&opts);
        assert!(
            rules.iter().any(|r| r.rule.id == "test/subdir-rule"),
            "subdirectory rule should be discovered"
        );
    }

    #[test]
    fn duplicate_id_last_write_wins() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Same id twice in different files — last-write (by HashMap) wins
        std::fs::write(
            dir.path().join("a_rule.json"),
            r#"{"id":"test/dup","family":"first","match":{}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b_rule.json"),
            r#"{"id":"test/dup","family":"second","match":{}}"#,
        )
        .unwrap();

        let opts = LoadRuleOptions {
            project_rules_dir: Some(dir.path().to_owned()),
            exclude_user: true,
            ..Default::default()
        };
        let rules = load_rules(&opts);
        let dups: Vec<_> = rules.iter().filter(|r| r.rule.id == "test/dup").collect();
        // There should be exactly one (deduped)
        assert_eq!(dups.len(), 1, "duplicate id should be deduplicated");
    }

    #[test]
    fn default_user_rules_dir_is_home_based() {
        // Just exercise the path: if home doesn't exist, should still not panic
        let path = super::user_rules_root(None);
        // Should end in .config/tokenjuice/rules
        assert!(path.to_string_lossy().contains("tokenjuice"));
    }

    #[test]
    fn default_project_rules_dir_is_cwd_based() {
        let path = super::project_rules_root(None, None);
        assert!(path.to_string_lossy().contains(".tokenjuice"));
    }
}
