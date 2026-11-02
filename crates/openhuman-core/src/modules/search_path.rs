//! Search-path loading that honours disabled `[components]` and host config.
//!
//! tinybus's `load_search_paths` admits every artifact in every directory, so a
//! disabled component's library would be mapped (and never unloaded) just by
//! sitting on the path. This walks the same directories but leaves out files
//! named after a disabled module's library, without opening them. TinyMCP is
//! also loaded separately because they require the host storage callback.

use std::path::Path;

use tinybus::module::{ModuleHost, ModuleInfo};

/// Modules that need host-supplied configuration cannot be admitted by
/// TinyBus's generic search-path loader, which initializes them with `{}`.
/// Leave those artifacts for the configured load below.
pub fn configured_module_lib_stems() -> Vec<String> {
    ["tinymcp", "tinyeventqueues", "tinysecurity"]
        .into_iter()
        .filter_map(super::registry::find)
        .flat_map(|record| {
            let stem = record.source.lib_name.to_string();
            [
                stem.clone(),
                format!("lib{stem}"),
                format!("{stem}_module"),
                format!("lib{stem}_module"),
            ]
        })
        .collect()
}

/// Modules whose first admission must include a retained host callback
/// binding. Their search-path artifacts remain available to the normal
/// `ensure_loaded` path, which installs that binding before loading.
fn callback_bound_module_lib_stems() -> Vec<String> {
    ["tinysecurity"]
        .into_iter()
        .filter_map(super::registry::find)
        .flat_map(|record| {
            let stem = record.source.lib_name.to_string();
            [
                stem.clone(),
                format!("lib{stem}"),
                format!("{stem}_module"),
                format!("lib{stem}_module"),
            ]
        })
        .collect()
}

fn should_defer_to_host_callback(path: &Path) -> bool {
    is_skipped(path, &callback_bound_module_lib_stems())
}

fn module_id_for_artifact(path: &Path) -> Option<&'static str> {
    let stem = path.file_stem()?.to_str()?;
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    let stem = stem.strip_suffix("_module").unwrap_or(stem);
    super::registry::ALL.iter().find_map(|record| {
        (record.id == stem || record.source.lib_name == stem).then_some(record.id)
    })
}

/// Whether `path` is a library file whose stem is in `skip`.
fn is_skipped(path: &Path, skip: &[String]) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| skip.iter().any(|s| s == stem))
}

/// Load search-path modules, supplying host config to modules that need it.
///
/// A directory with no disabled or configured modules goes through `load_dir`
/// unchanged, which keeps tinybus's dependency ordering. A directory holding
/// either is loaded file by file, with host config for configured modules.
pub async fn load_enabled<F, Fut>(
    host: &ModuleHost,
    skip: &[String],
    module_config: F,
) -> Vec<Result<ModuleInfo, tinybus::Error>>
where
    F: Fn(&str) -> Fut,
    Fut: std::future::Future<Output = serde_json::Value>,
{
    let disabled = skip.to_vec();
    let configured = configured_module_lib_stems();
    let callback_bound = callback_bound_module_lib_stems();
    let mut skip = disabled.clone();
    skip.extend(configured.iter().cloned());
    skip.extend(callback_bound.iter().cloned());
    skip.sort();
    skip.dedup();
    if skip.is_empty() {
        return host.load_search_paths();
    }
    let mut outcomes = Vec::new();
    for dir in host.search_paths().into_iter().filter(|p| p.is_dir()) {
        let mut files: Vec<_> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => Vec::new(),
        };
        files.sort();
        let has_skipped = files.iter().any(|f| is_skipped(f, &skip));
        if !has_skipped {
            match host.load_dir(&dir) {
                Ok(results) => outcomes.extend(results),
                Err(error) => outcomes.push(Err(error)),
            }
            continue;
        }
        for file in files {
            if is_skipped(&file, &disabled) {
                log::debug!("[modules] search-path artifact skipped: component disabled");
                continue;
            }
            if is_skipped(&file, &callback_bound) {
                log::debug!("[modules] callback-bound module deferred until requested");
                continue;
            }
            if is_skipped(&file, &configured) {
                if let Some(id) = module_id_for_artifact(&file) {
                    outcomes.push(host.load_file_with_config(&file, module_config(id).await));
                    continue;
                }
                log::debug!("[modules] configured module artifact has no registry entry");
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
