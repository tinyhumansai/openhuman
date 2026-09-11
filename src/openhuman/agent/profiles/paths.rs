//! Personality-scoped path resolution and context for multi-agent sessions.

use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use super::home::validate_profile_id;
use super::types::AgentProfile;

/// Reject path strings that could escape the workspace: absolute paths,
/// root/prefix components, or any `..` segment.
fn is_safe_relative_path(rel: &Path) -> bool {
    !rel.is_absolute()
        && rel.components().all(|c| {
            !matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

/// Resolve the memory subdirectory name for a given suffix.
/// `""` → `"memory"`, `"-1"` → `"memory-1"`, `"-2"` → `"memory-2"`.
pub fn memory_subdir_for_suffix(suffix: &str) -> String {
    if suffix.is_empty() {
        "memory".to_string()
    } else {
        format!("memory{suffix}")
    }
}

/// Resolve the memory_tree subdirectory name for a given suffix.
pub fn memory_tree_subdir_for_suffix(suffix: &str) -> String {
    if suffix.is_empty() {
        "memory_tree".to_string()
    } else {
        format!("memory_tree{suffix}")
    }
}

/// Resolve the session_raw subdirectory name for a given suffix.
pub fn session_raw_subdir_for_suffix(suffix: &str) -> String {
    if suffix.is_empty() {
        "session_raw".to_string()
    } else {
        format!("session_raw{suffix}")
    }
}

/// The workspace file a profile's `soul_md_path` points at, or `None` when
/// the profile names no file, the relative path is unsafe, or it escapes the
/// workspace after canonicalization. Shared by [`resolve_personality_soul`]
/// (which reads it) and the channel runtime's identity fingerprint (which
/// stats it) so both agree on what "the soul file" is.
///
/// Synchronous fs calls are intentional — this runs during prompt construction
/// on a blocking thread and the workspace is always local disk.
pub(crate) fn soul_md_file_path(workspace_dir: &Path, profile: &AgentProfile) -> Option<PathBuf> {
    let rel_path = profile.soul_md_path.as_deref()?;
    let rel = Path::new(rel_path);
    if !is_safe_relative_path(rel) {
        tracing::debug!(
            profile_id = %profile.id,
            soul_md_path = %rel_path,
            "[personality] rejected unsafe soul_md_path, trying inline"
        );
        return None;
    }
    let path = workspace_dir.join(rel);
    // Guard against symlink traversal: a symlink inside the workspace can
    // point outside it. Canonicalize both sides and reject if the resolved
    // path escapes the workspace root.
    if let (Ok(canonical_ws), Ok(canonical_p)) = (workspace_dir.canonicalize(), path.canonicalize())
    {
        if !canonical_p.starts_with(&canonical_ws) {
            tracing::warn!(
                path = %path.display(),
                profile_id = %profile.id,
                "[personality] soul_md_path escapes workspace after canonicalization, trying inline"
            );
            return None;
        }
    }
    Some(path)
}

/// Resolve the SOUL.md content for a personality.
///
/// Resolution order (hermes-style — the per-profile identity file wins and is
/// re-read on every prompt build):
/// 1. `personalities/<id>/SOUL.md` — the canonical per-profile identity file
///    (skipped when the profile id fails [`validate_profile_id`], so legacy
///    profiles with arbitrary ids can't construct an unexpected path).
/// 2. `soul_md_path` — read the file at that relative path under workspace.
/// 3. `soul_md` — inline content from the profile.
/// 4. `None` — caller falls back to the workspace root `SOUL.md`.
pub fn resolve_personality_soul(workspace_dir: &Path, profile: &AgentProfile) -> Option<String> {
    // Step 1: the per-profile home SOUL.md. Only attempted for ids that pass the
    // hermes name grammar — a legacy/built-in id that fails validation skips
    // straight to the existing (2)/(3)/(4) resolution below, unchanged.
    match validate_profile_id(&profile.id) {
        Ok(()) => {
            let home_soul = super::home::profile_home(workspace_dir, &profile.id).join("SOUL.md");
            match std::fs::read_to_string(&home_soul) {
                Ok(content) if !content.trim().is_empty() => {
                    tracing::debug!(
                        path = %home_soul.display(),
                        profile_id = %profile.id,
                        "[personality] soul_md loaded from profile home"
                    );
                    return Some(content);
                }
                Ok(_) => {
                    tracing::debug!(
                        profile_id = %profile.id,
                        "[personality] profile-home SOUL.md empty, trying soul_md_path/inline"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        path = %home_soul.display(),
                        profile_id = %profile.id,
                        error = %e,
                        "[personality] profile-home SOUL.md absent, trying soul_md_path/inline"
                    );
                }
            }
        }
        Err(e) => {
            tracing::debug!(
                profile_id = %profile.id,
                error = %e,
                "[personality] profile id fails validation, skipping profile-home SOUL.md"
            );
        }
    }

    if profile.soul_md_path.is_some() {
        // An unsafe or escaping path is rejected (logged by the resolver) and
        // falls through to the inline soul, as before.
        let Some(path) = soul_md_file_path(workspace_dir, profile) else {
            return profile
                .soul_md
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned();
        };
        match std::fs::read_to_string(&path) {
            Ok(content) if !content.trim().is_empty() => {
                tracing::debug!(
                    path = %path.display(),
                    profile_id = %profile.id,
                    "[personality] soul_md loaded from file"
                );
                return Some(content);
            }
            Ok(_) => {
                tracing::debug!(
                    path = %path.display(),
                    profile_id = %profile.id,
                    "[personality] soul_md_path file empty, trying inline"
                );
            }
            Err(e) => {
                tracing::debug!(
                    path = %path.display(),
                    profile_id = %profile.id,
                    error = %e,
                    "[personality] soul_md_path read failed, trying inline"
                );
            }
        }
    }

    if let Some(ref inline) = profile.soul_md {
        if !inline.trim().is_empty() {
            tracing::debug!(
                profile_id = %profile.id,
                len = inline.len(),
                "[personality] soul_md loaded from inline"
            );
            return Some(inline.clone());
        }
    }

    tracing::debug!(
        profile_id = %profile.id,
        "[personality] no personality-specific soul_md, falling back to root"
    );
    None
}

/// Resolve a personality's MEMORY.md content.
///
/// Looks for `personalities/{profile_id}/MEMORY.md` under the workspace.
/// Returns `None` if the file doesn't exist or is empty — caller falls
/// back to the workspace root `MEMORY.md`.
pub fn resolve_personality_memory_md(
    workspace_dir: &Path,
    profile: &AgentProfile,
) -> Option<String> {
    let path = workspace_dir
        .join("personalities")
        .join(&profile.id)
        .join("MEMORY.md");
    match std::fs::read_to_string(&path) {
        Ok(content) if !content.trim().is_empty() => {
            tracing::debug!(
                path = %path.display(),
                profile_id = %profile.id,
                "[personality] memory_md loaded from personality dir"
            );
            Some(content)
        }
        _ => None,
    }
}

/// Fingerprint every profile input baked into a cached session agent.
///
/// The profile record alone is insufficient because users may edit the
/// canonical SOUL.md or MEMORY.md files directly. Hashing their resolved
/// contents makes the next web-chat turn rebuild its cached agent without
/// retaining the files themselves in cache metadata or logs.
pub fn profile_session_signature(workspace_dir: &Path, profile: &AgentProfile) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    super::types::profile_signature(profile).hash(&mut hasher);
    resolve_personality_soul(workspace_dir, profile).hash(&mut hasher);
    resolve_personality_memory_md(workspace_dir, profile).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Derive the effective memory directory suffix for a profile.
///
/// Precedence:
/// 1. when `dedicated_memory` is set, derive `"-<id>"` from the profile id
///    (id must pass [`validate_profile_id`], else fall back to the shared `""`
///    and warn — a legacy id can't mint an unexpected directory name). This is
///    an explicit user opt-in and **wins over** the auto-assigned numeric
///    suffix: the store stamps every non-default profile with `Some("-1")`,
///    `Some("-2")`, … on upsert, so if the numeric suffix took precedence the
///    isolation toggle could never take effect (it would be dead code). Toggling
///    `dedicated_memory` on therefore switches the profile to its own
///    `memory-<id>` subtree — the intended behaviour of the toggle.
/// 2. else, an explicit `memory_dir_suffix` (the legacy auto-assigned numeric
///    suffix, e.g. `"-1"`) — pre-existing non-dedicated profiles keep their
///    directories;
/// 3. else `""` (the shared/global memory tree).
///
/// The returned suffix feeds the existing
/// [`memory_subdir_for_suffix`] / [`memory_tree_subdir_for_suffix`] /
/// [`session_raw_subdir_for_suffix`] helpers unchanged.
pub fn effective_memory_suffix(profile: &AgentProfile) -> String {
    if profile.dedicated_memory {
        match validate_profile_id(&profile.id) {
            Ok(()) => {
                let suffix = format!("-{}", profile.id);
                tracing::debug!(
                    profile_id = %profile.id,
                    suffix = %suffix,
                    "[personality] effective_memory_suffix derived from dedicated_memory"
                );
                return suffix;
            }
            Err(e) => {
                tracing::warn!(
                    profile_id = %profile.id,
                    error = %e,
                    "[personality] dedicated_memory requested but id fails validation, \
                     falling back to legacy/shared memory tree"
                );
            }
        }
    }
    if let Some(suffix) = profile
        .memory_dir_suffix
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        tracing::debug!(
            profile_id = %profile.id,
            suffix = %suffix,
            "[personality] effective_memory_suffix using legacy numeric suffix"
        );
        return suffix.to_string();
    }
    tracing::debug!(
        profile_id = %profile.id,
        "[personality] effective_memory_suffix using shared memory tree"
    );
    String::new()
}

/// All personality-resolved overrides needed to build a scoped agent session.
#[derive(Debug, Clone)]
pub struct PersonalityContext {
    pub profile: AgentProfile,
    pub memory_suffix: String,
    pub soul_md_override: Option<String>,
    pub memory_md_override: Option<String>,
    pub composio_allowlist: Option<Vec<String>>,
    pub voice_id: Option<String>,
}

impl PersonalityContext {
    /// Build from a resolved `AgentProfile`, reading personality files from the workspace.
    pub fn from_profile(workspace_dir: &Path, profile: AgentProfile) -> Self {
        let memory_suffix = effective_memory_suffix(&profile);
        let soul_md_override = resolve_personality_soul(workspace_dir, &profile);
        let memory_md_override = resolve_personality_memory_md(workspace_dir, &profile);
        let composio_allowlist = profile.composio_integrations.clone();
        let voice_id = profile.voice_id.clone();

        Self {
            profile,
            memory_suffix,
            soul_md_override,
            memory_md_override,
            composio_allowlist,
            voice_id,
        }
    }
}

/// Filter connected integrations by an allowlist of toolkit slugs.
///
/// - `None` → passthrough (all integrations).
/// - `Some([])` → no integrations.
/// - `Some(["slack", "gmail"])` → only those toolkits.
pub fn filter_integrations<T: Clone + HasToolkit>(
    all: &[T],
    allowlist: Option<&[String]>,
) -> Vec<T> {
    match allowlist {
        None => all.to_vec(),
        Some(allowed) => all
            .iter()
            .filter(|ci| {
                allowed
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(ci.toolkit_name()))
            })
            .cloned()
            .collect(),
    }
}

/// Trait to abstract over integration types that have a toolkit name.
pub trait HasToolkit {
    fn toolkit_name(&self) -> &str;
}

impl HasToolkit for crate::openhuman::agent::prompts::ConnectedIntegration {
    fn toolkit_name(&self) -> &str {
        &self.toolkit
    }
}

#[cfg(test)]
#[path = "paths_tests.rs"]
mod tests;
