//! Source discovery: which legacy session files exist in a workspace.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::types::SourceKind;

/// One discovered source, keyed by session stem.
#[derive(Debug, Clone)]
pub struct SourceItem {
    pub stem: String,
    pub kind: SourceKind,
    /// Absolute path of the file the messages will be read from.
    pub path: PathBuf,
    /// Workspace-relative path of `path`, for descriptors and the ledger.
    pub relative: String,
    /// Workspace-relative Markdown companion, when one exists next to a
    /// JSONL source (informational only — never read when JSONL exists).
    pub md_companion: Option<String>,
}

/// Scan `session_raw/` (flat + legacy `DDMMYYYY` folders) and `sessions/`
/// Markdown directories. Returns items sorted by stem, plus scan warnings.
///
/// Precedence per stem: flat JSONL > legacy-dir JSONL > Markdown-only. The
/// same stem never yields two items.
pub fn discover_sources(workspace: &Path) -> (Vec<SourceItem>, Vec<String>) {
    let mut warnings = Vec::new();
    // stem → item, first writer wins per the precedence order below.
    let mut by_stem: BTreeMap<String, SourceItem> = BTreeMap::new();

    let raw_dir = workspace.join("session_raw");

    // 1. Flat JSONL (current layout).
    for path in list_files(&raw_dir, "jsonl", &mut warnings) {
        insert_stem(
            &mut by_stem,
            workspace,
            path,
            SourceKind::Jsonl,
            &mut warnings,
        );
    }

    // 2. Legacy date-folder JSONL.
    for sub in list_dirs(&raw_dir, &mut warnings) {
        let name = sub
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !is_ddmmyyyy(&name) {
            continue;
        }
        for path in list_files(&sub, "jsonl", &mut warnings) {
            insert_stem(
                &mut by_stem,
                workspace,
                path,
                SourceKind::JsonlLegacyDir,
                &mut warnings,
            );
        }
    }

    // 3. Markdown sessions: only stems with no JSONL anywhere. Also record
    //    companions for stems that do have JSONL.
    let sessions_dir = workspace.join("sessions");
    for sub in list_dirs(&sessions_dir, &mut warnings) {
        for path in list_files(&sub, "md", &mut warnings) {
            let Some(stem) = file_stem(&path) else {
                continue;
            };
            let relative = relative_to(workspace, &path);
            match by_stem.get_mut(&stem) {
                Some(item) => {
                    if item.md_companion.is_none() {
                        item.md_companion = Some(relative);
                    }
                }
                None => {
                    by_stem.insert(
                        stem.clone(),
                        SourceItem {
                            stem,
                            kind: SourceKind::Markdown,
                            relative,
                            path,
                            md_companion: None,
                        },
                    );
                }
            }
        }
    }

    (by_stem.into_values().collect(), warnings)
}

fn insert_stem(
    by_stem: &mut BTreeMap<String, SourceItem>,
    workspace: &Path,
    path: PathBuf,
    kind: SourceKind,
    warnings: &mut Vec<String>,
) {
    let Some(stem) = file_stem(&path) else {
        warnings.push(format!("unreadable file name, skipped: {}", path.display()));
        return;
    };
    let relative = relative_to(workspace, &path);
    if let Some(existing) = by_stem.get(&stem) {
        warnings.push(format!(
            "duplicate stem '{stem}': keeping {}, ignoring {relative}",
            existing.relative
        ));
        return;
    }
    by_stem.insert(
        stem.clone(),
        SourceItem {
            stem,
            kind,
            relative,
            path,
            md_companion: None,
        },
    );
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem().map(|s| s.to_string_lossy().to_string())
}

fn relative_to(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn list_files(dir: &Path, ext: &str, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    list_entries(dir, warnings)
        .into_iter()
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == ext))
        .collect()
}

fn list_dirs(dir: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    list_entries(dir, warnings)
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
}

fn list_entries(dir: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    if !dir.exists() {
        return Vec::new();
    }
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let mut paths: Vec<PathBuf> =
                entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            paths.sort();
            paths
        }
        Err(err) => {
            warnings.push(format!("cannot read {}: {err}", dir.display()));
            Vec::new()
        }
    }
}

/// Exactly 8 ASCII digits — the legacy `DDMMYYYY` folder convention (matches
/// `session::migration::is_ddmmyyyy`).
fn is_ddmmyyyy(name: &str) -> bool {
    name.len() == 8 && name.bytes().all(|b| b.is_ascii_digit())
}
