//! Search-path loading that honours disabled `[components]`.
//!
//! tinybus's `load_search_paths` admits every artifact in every directory, so a
//! disabled component's library would be mapped (and never unloaded) just by
//! sitting on the path. This walks the same directories but leaves out files
//! named after a disabled module's library, without opening them.

use std::path::Path;

use tinybus::module::{ModuleHost, ModuleInfo};

/// Whether `path` is a library file whose stem is in `skip`.
fn is_skipped(path: &Path, skip: &[String]) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| skip.iter().any(|s| s == stem))
}

/// [`ModuleHost::load_search_paths`], minus artifacts named in `skip`.
///
/// A directory with nothing to skip goes through `load_dir` unchanged, which
/// keeps tinybus's dependency ordering. Only a directory holding a skipped
/// artifact is loaded file by file.
pub fn load_enabled(host: &ModuleHost, skip: &[String]) -> Vec<Result<ModuleInfo, tinybus::Error>> {
    if skip.is_empty() {
        return host.load_search_paths();
    }
    let mut outcomes = Vec::new();
    for dir in ModuleHost::search_paths()
        .into_iter()
        .filter(|p| p.is_dir())
    {
        let mut files: Vec<_> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => Vec::new(),
        };
        files.sort();
        let has_skipped = files.iter().any(|f| is_skipped(f, skip));
        if !has_skipped {
            match host.load_dir(&dir) {
                Ok(results) => outcomes.extend(results),
                Err(error) => outcomes.push(Err(error)),
            }
            continue;
        }
        for file in files {
            if is_skipped(&file, skip) {
                log::debug!("[modules] search-path artifact skipped: component disabled");
                continue;
            }
            if file
                .extension()
                .is_some_and(|e| matches!(e.to_str(), Some("so" | "dylib" | "dll")))
            {
                outcomes.push(host.load_file(&file));
            }
        }
    }
    outcomes
}

#[cfg(test)]
#[path = "search_path_tests.rs"]
mod tests;
