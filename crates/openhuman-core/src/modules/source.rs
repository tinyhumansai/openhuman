//! Which source serves a module: release, linked, source, or a local file.
//!
//! The decision is a pure function of configuration plus two facts about the
//! build ([`linked_factory_available`], [`source_artifact`]), so every mode and
//! every fallback is testable without loading anything. `release` is the
//! default everywhere and the only mode CI and shipped builds use; `linked` and
//! `source` exist for local development and local tests.

use std::path::{Path, PathBuf};

use super::types::ModuleSource;
use crate::config::schema::ModuleSourceMode;
use crate::config::Config;

/// The outcome of choosing a source for one module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SourcePlan {
    /// What the configuration asked for.
    pub requested: ModuleSourceMode,
    /// What will actually serve the module.
    pub served: ModuleSource,
    /// The local artifact to load, for [`ModuleSource::LocalFile`] and
    /// [`ModuleSource::Source`].
    pub artifact: Option<PathBuf>,
    /// Why `served` differs from `requested`. A static string: it ends up in
    /// `ModuleStatus.detail`, so it never carries a path, URL or credential.
    pub fallback: Option<&'static str>,
}

/// Whether this binary registered an in-process factory for `id`.
///
/// Only a binary whose host called [`super::linked::register_linked`] (the CLI
/// built with `linked-modules`) has one; every other binary falls back.
#[must_use]
pub(super) fn linked_factory_available(id: &str) -> bool {
    #[cfg(feature = "modules")]
    {
        super::linked::is_registered(id)
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = id;
        false
    }
}

/// Environment variable naming the directory `scripts/modules/build.mjs`
/// wrote its artifacts under (`<dir>/<id>/<library>` plus a `modules.toml`
/// carrying the operator-trust digest). The build step prints it.
pub(crate) const SOURCE_DIR_ENV: &str = "OPENHUMAN_MODULE_SOURCE_DIR";

/// The locally built artifact for `id`, if one exists.
///
/// Looks under [`SOURCE_DIR_ENV`]; with the variable unset, or no built file
/// there, a `source` request without an explicit path falls back to `release`.
#[must_use]
pub(super) fn source_artifact(id: &str) -> Option<PathBuf> {
    let root = std::env::var_os(SOURCE_DIR_ENV)?;
    artifact_in(Path::new(&root), id)
}

/// The built library for `id` under `root`, if the file exists.
///
/// The file name comes from the registry's `SourceSpec`, so the caller never
/// supplies one and an unknown id has no artifact.
#[must_use]
pub(super) fn artifact_in(root: &Path, id: &str) -> Option<PathBuf> {
    let record = super::registry::find(id)?;
    let path = root.join(record.id).join(record.source.artifact_filename());
    path.is_file().then_some(path)
}

/// A real module library for `id` on this machine, for tests that need one.
///
/// Looks first under [`SOURCE_DIR_ENV`] (a `scripts/modules/build.mjs` build),
/// then in every `OPENHUMAN_MODULE_PATH` directory (the released modules the
/// CI installer stages). `None` means the test should skip, not fail: no module
/// was provided. Never used by the loader, which goes through [`plan`].
#[doc(hidden)]
#[must_use]
pub fn local_artifact(id: &str) -> Option<PathBuf> {
    if let Some(path) = source_artifact(id) {
        return Some(path);
    }
    super::registry::find(id)?;
    let dirs = std::env::var_os("OPENHUMAN_MODULE_PATH")?;
    std::env::split_paths(&dirs).find_map(|dir| module_artifact_in(&dir, id))
}

/// Find either a locally built library or a released TinyBus module in one
/// directory. Source builds use the crate library name; release archives use
/// the module ABI name, which adds the `_module` suffix.
fn module_artifact_in(dir: &Path, id: &str) -> Option<PathBuf> {
    let record = super::registry::find(id)?;
    let source = dir.join(record.source.artifact_filename());
    if source.is_file() {
        return Some(source);
    }

    let released_name = if cfg!(target_os = "windows") {
        format!("{id}_module.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{id}_module.dylib")
    } else {
        format!("lib{id}_module.so")
    };
    let released = dir.join(released_name);
    released.is_file().then_some(released)
}

/// Environment variable listing module ids whose gitlink is ahead of the
/// registry pin, comma separated. `scripts/modules/pin-status.mjs --print-env`
/// prints it; a developer exports it next to `OPENHUMAN_MODULE_SOURCE`.
pub(crate) const PIN_AHEAD_ENV: &str = "OPENHUMAN_MODULE_PIN_AHEAD";

/// The warning for serving `id` from a local build whose gitlink is ahead of
/// its released pin, or `None` when there is nothing to say.
///
/// Pure: `ahead_ids` is the raw [`PIN_AHEAD_ENV`] value and `debug` is whether
/// this is a debug build. Only `Linked` and `Source` serve a local build, and
/// release builds stay silent.
#[must_use]
pub(super) fn pin_ahead_warning(
    id: &str,
    served: ModuleSource,
    ahead_ids: Option<&str>,
    debug: bool,
) -> Option<String> {
    if !debug || !matches!(served, ModuleSource::Linked | ModuleSource::Source) {
        return None;
    }
    let ahead = ahead_ids?
        .split(',')
        .any(|candidate| candidate.trim() == id);
    ahead.then(|| {
        format!(
            "[modules] id={id} served={served:?}: gitlink ahead of released pin; you are running a locally compiled module, publish a release and re-pin (scripts/modules/release-and-repin.mjs) before CI will pass"
        )
    })
}

/// Choose the source for `id` under `config`.
pub(super) fn plan(config: &Config, id: &str) -> SourcePlan {
    plan_with(
        config,
        id,
        linked_factory_available(id),
        source_artifact(id),
    )
}

/// [`plan`] with the build facts supplied, for tests.
pub(super) fn plan_with(
    config: &Config,
    id: &str,
    linked_available: bool,
    built_artifact: Option<PathBuf>,
) -> SourcePlan {
    let mut mode = config.modules.source;
    if let Some(entry) = config.modules.overrides.iter().find(|entry| entry.id == id) {
        match entry.validate() {
            Err(reason) => {
                log::warn!("[modules] ignoring override for '{id}': {reason}");
            }
            Ok(()) => {
                if let Some(path) = &entry.path {
                    return SourcePlan {
                        requested: ModuleSourceMode::Source,
                        served: ModuleSource::LocalFile,
                        artifact: Some(PathBuf::from(path)),
                        fallback: None,
                    };
                }
                if let Some(source) = entry.source {
                    mode = source;
                }
            }
        }
    }

    let released = |fallback: &'static str| SourcePlan {
        requested: mode,
        served: ModuleSource::Release,
        artifact: None,
        fallback: Some(fallback),
    };
    match mode {
        ModuleSourceMode::Release => SourcePlan {
            requested: mode,
            served: ModuleSource::Release,
            artifact: None,
            fallback: None,
        },
        ModuleSourceMode::Linked if linked_available => SourcePlan {
            requested: mode,
            served: ModuleSource::Linked,
            artifact: None,
            fallback: None,
        },
        ModuleSourceMode::Linked => {
            released("no linked factory is registered in this binary; serving the release")
        }
        ModuleSourceMode::Source => match built_artifact {
            Some(path) => SourcePlan {
                requested: mode,
                served: ModuleSource::Source,
                artifact: Some(path),
                fallback: None,
            },
            None => released("no source-built artifact is available; serving the release"),
        },
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
