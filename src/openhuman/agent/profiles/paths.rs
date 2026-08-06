//! Personality-scoped path resolution and context for multi-agent sessions.

use std::hash::{Hash, Hasher};
use std::path::{Component, Path};

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

    if let Some(ref rel_path) = profile.soul_md_path {
        let rel = Path::new(rel_path);
        if !is_safe_relative_path(rel) {
            tracing::debug!(
                profile_id = %profile.id,
                soul_md_path = %rel_path,
                "[personality] rejected unsafe soul_md_path, trying inline"
            );
            // Fall through to inline check below.
            return profile
                .soul_md
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned();
        }
        let path = workspace_dir.join(rel);
        // Guard against symlink traversal: a symlink inside the workspace can
        // point outside it. Canonicalize both sides and reject if the resolved
        // path escapes the workspace root.
        // Note: synchronous fs calls here are intentional — soul_md is loaded
        // during prompt construction on a tokio blocking thread; the workspace
        // is always local disk (never a remote mount).
        if let (Ok(canonical_ws), Ok(canonical_p)) =
            (workspace_dir.canonicalize(), path.canonicalize())
        {
            if !canonical_p.starts_with(&canonical_ws) {
                tracing::warn!(
                    path = %path.display(),
                    profile_id = %profile.id,
                    "[personality] soul_md_path escapes workspace after canonicalization, trying inline"
                );
                return profile
                    .soul_md
                    .as_ref()
                    .filter(|s| !s.trim().is_empty())
                    .cloned();
            }
        }
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_profile(id: &str) -> AgentProfile {
        AgentProfile {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            agent_id: "orchestrator".to_string(),
            model_override: None,
            temperature: None,
            system_prompt_suffix: None,
            allowed_tools: None,
            built_in: false,
            avatar_url: None,
            voice_id: None,
            soul_md: None,
            soul_md_path: None,
            composio_integrations: None,
            memory_sources: None,
            include_agent_conversations: true,
            allowed_skills: None,
            allowed_mcp_servers: None,
            memory_dir_suffix: None,
            is_master: false,
            sort_order: None,
            dedicated_memory: false,
            dedicated_workspace: false,
        }
    }

    #[test]
    fn memory_subdir_for_suffix_patterns() {
        assert_eq!(memory_subdir_for_suffix(""), "memory");
        assert_eq!(memory_subdir_for_suffix("-1"), "memory-1");
        assert_eq!(memory_subdir_for_suffix("-2"), "memory-2");
        assert_eq!(memory_subdir_for_suffix("-10"), "memory-10");
    }

    #[test]
    fn memory_tree_subdir_for_suffix_patterns() {
        assert_eq!(memory_tree_subdir_for_suffix(""), "memory_tree");
        assert_eq!(memory_tree_subdir_for_suffix("-1"), "memory_tree-1");
    }

    #[test]
    fn session_raw_subdir_for_suffix_patterns() {
        assert_eq!(session_raw_subdir_for_suffix(""), "session_raw");
        assert_eq!(session_raw_subdir_for_suffix("-1"), "session_raw-1");
    }

    #[test]
    fn resolve_soul_prefers_profile_home_file() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("personalities").join("alice");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("SOUL.md"), "Home identity for Alice").unwrap();

        let mut profile = test_profile("alice");
        // Both inline and soul_md_path present — the profile-home file still wins.
        profile.soul_md = Some("Inline soul".to_string());
        let result = resolve_personality_soul(tmp.path(), &profile);
        assert_eq!(result.as_deref(), Some("Home identity for Alice"));
    }

    #[test]
    fn resolve_soul_skips_home_file_for_invalid_legacy_id() {
        let tmp = TempDir::new().unwrap();
        // A legacy id that fails validate_profile_id (space + uppercase).
        let legacy_id = "Legacy Id";
        let home = tmp.path().join("personalities").join(legacy_id);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("SOUL.md"), "SHOULD BE SKIPPED").unwrap();

        let mut profile = test_profile("placeholder");
        profile.id = legacy_id.to_string();
        profile.soul_md = Some("Legacy inline soul".to_string());
        // Step 1 (profile-home SOUL.md) is skipped for the invalid id; resolution
        // falls through to the inline value.
        let result = resolve_personality_soul(tmp.path(), &profile);
        assert_eq!(result.as_deref(), Some("Legacy inline soul"));
    }

    #[test]
    fn resolve_soul_empty_home_file_falls_through_to_inline() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("personalities").join("alice");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("SOUL.md"), "   \n").unwrap(); // whitespace only

        let mut profile = test_profile("alice");
        profile.soul_md = Some("Inline wins over empty home file".to_string());
        let result = resolve_personality_soul(tmp.path(), &profile);
        assert_eq!(result.as_deref(), Some("Inline wins over empty home file"));
    }

    #[test]
    fn effective_memory_suffix_dedicated_wins_over_numeric() {
        let mut profile = test_profile("alice");
        // The store auto-assigns a numeric suffix to every non-default profile,
        // so `dedicated_memory` must win over it — otherwise the toggle would be
        // dead code and could never route to the `memory-<id>` subtree.
        profile.memory_dir_suffix = Some("-3".to_string());
        profile.dedicated_memory = true;
        assert_eq!(effective_memory_suffix(&profile), "-alice");
    }

    #[test]
    fn effective_memory_suffix_numeric_retained_when_not_dedicated() {
        let mut profile = test_profile("alice");
        // With dedicated_memory off, the persisted legacy numeric suffix is
        // retained so an existing memory directory is never orphaned.
        profile.memory_dir_suffix = Some("-3".to_string());
        profile.dedicated_memory = false;
        assert_eq!(effective_memory_suffix(&profile), "-3");
    }

    #[test]
    fn effective_memory_suffix_invalid_id_dedicated_falls_back_to_numeric() {
        let mut profile = test_profile("placeholder");
        profile.id = "Bad Id".to_string();
        // An invalid id can't mint a `-<id>` directory even with dedicated on, so
        // it falls back to the persisted numeric suffix rather than the shared
        // tree.
        profile.memory_dir_suffix = Some("-2".to_string());
        profile.dedicated_memory = true;
        assert_eq!(effective_memory_suffix(&profile), "-2");
    }

    #[test]
    fn effective_memory_suffix_dedicated_derives_from_id() {
        let mut profile = test_profile("alice");
        profile.memory_dir_suffix = None;
        profile.dedicated_memory = true;
        assert_eq!(effective_memory_suffix(&profile), "-alice");
    }

    #[test]
    fn effective_memory_suffix_shared_default() {
        let mut profile = test_profile("alice");
        profile.memory_dir_suffix = None;
        profile.dedicated_memory = false;
        assert_eq!(effective_memory_suffix(&profile), "");
    }

    #[test]
    fn effective_memory_suffix_invalid_id_falls_back_to_shared() {
        let mut profile = test_profile("placeholder");
        profile.id = "Bad Id".to_string();
        profile.memory_dir_suffix = None;
        profile.dedicated_memory = true;
        // Invalid id cannot mint a directory name — fall back to shared "".
        assert_eq!(effective_memory_suffix(&profile), "");
    }

    #[test]
    fn effective_memory_suffix_empty_string_suffix_is_not_legacy() {
        let mut profile = test_profile("alice");
        // The default profile stores Some("") — treated as "no legacy suffix", so
        // dedicated_memory (if set) still derives, else shared.
        profile.memory_dir_suffix = Some(String::new());
        profile.dedicated_memory = false;
        assert_eq!(effective_memory_suffix(&profile), "");
    }

    #[test]
    fn resolve_soul_inline_fallback() {
        let tmp = TempDir::new().unwrap();
        let mut profile = test_profile("alice");
        profile.soul_md = Some("I am Alice, a friendly assistant.".to_string());
        let result = resolve_personality_soul(tmp.path(), &profile);
        assert_eq!(result.as_deref(), Some("I am Alice, a friendly assistant."));
    }

    #[test]
    fn resolve_soul_file_takes_precedence() {
        let tmp = TempDir::new().unwrap();
        let soul_path = tmp.path().join("souls").join("alice.md");
        std::fs::create_dir_all(soul_path.parent().unwrap()).unwrap();
        std::fs::write(&soul_path, "File-based soul").unwrap();

        let mut profile = test_profile("alice");
        profile.soul_md_path = Some("souls/alice.md".to_string());
        profile.soul_md = Some("Inline soul".to_string());
        let result = resolve_personality_soul(tmp.path(), &profile);
        assert_eq!(result.as_deref(), Some("File-based soul"));
    }

    #[test]
    fn resolve_soul_returns_none_when_empty() {
        let tmp = TempDir::new().unwrap();
        let profile = test_profile("alice");
        let result = resolve_personality_soul(tmp.path(), &profile);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_memory_md_from_personality_dir() {
        let tmp = TempDir::new().unwrap();
        let mem_path = tmp
            .path()
            .join("personalities")
            .join("alice")
            .join("MEMORY.md");
        std::fs::create_dir_all(mem_path.parent().unwrap()).unwrap();
        std::fs::write(&mem_path, "Alice remembers things.").unwrap();

        let profile = test_profile("alice");
        let result = resolve_personality_memory_md(tmp.path(), &profile);
        assert_eq!(result.as_deref(), Some("Alice remembers things."));
    }

    #[test]
    fn resolve_memory_md_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        let profile = test_profile("alice");
        let result = resolve_personality_memory_md(tmp.path(), &profile);
        assert!(result.is_none());
    }

    #[test]
    fn profile_session_signature_tracks_profile_file_edits() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("personalities/alice");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("SOUL.md"), "first soul").unwrap();
        std::fs::write(home.join("MEMORY.md"), "first memory").unwrap();
        let profile = test_profile("alice");

        let original = profile_session_signature(tmp.path(), &profile);
        std::fs::write(home.join("SOUL.md"), "second soul").unwrap();
        let after_soul_edit = profile_session_signature(tmp.path(), &profile);
        assert_ne!(original, after_soul_edit);

        std::fs::write(home.join("MEMORY.md"), "second memory").unwrap();
        let after_memory_edit = profile_session_signature(tmp.path(), &profile);
        assert_ne!(after_soul_edit, after_memory_edit);
    }

    #[test]
    fn personality_context_from_profile() {
        let tmp = TempDir::new().unwrap();
        let mut profile = test_profile("bob");
        profile.memory_dir_suffix = Some("-1".to_string());
        profile.voice_id = Some("voice-xyz".to_string());
        profile.composio_integrations = Some(vec!["slack".to_string()]);
        profile.soul_md = Some("I am Bob.".to_string());

        let ctx = PersonalityContext::from_profile(tmp.path(), profile);
        assert_eq!(ctx.memory_suffix, "-1");
        assert_eq!(ctx.voice_id.as_deref(), Some("voice-xyz"));
        assert_eq!(ctx.soul_md_override.as_deref(), Some("I am Bob."));
        assert_eq!(ctx.composio_allowlist.as_ref().unwrap(), &["slack"]);
    }

    #[derive(Clone)]
    struct FakeIntegration {
        toolkit: String,
    }
    impl HasToolkit for FakeIntegration {
        fn toolkit_name(&self) -> &str {
            &self.toolkit
        }
    }

    #[test]
    fn filter_integrations_none_passthrough() {
        let all = vec![
            FakeIntegration {
                toolkit: "slack".into(),
            },
            FakeIntegration {
                toolkit: "gmail".into(),
            },
        ];
        let filtered = filter_integrations(&all, None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_integrations_allowlist() {
        let all = vec![
            FakeIntegration {
                toolkit: "slack".into(),
            },
            FakeIntegration {
                toolkit: "gmail".into(),
            },
            FakeIntegration {
                toolkit: "notion".into(),
            },
        ];
        let allowed = vec!["slack".to_string(), "notion".to_string()];
        let filtered = filter_integrations(&all, Some(&allowed));
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].toolkit, "slack");
        assert_eq!(filtered[1].toolkit, "notion");
    }

    #[test]
    fn filter_integrations_empty_allowlist() {
        let all = vec![FakeIntegration {
            toolkit: "slack".into(),
        }];
        let allowed: Vec<String> = vec![];
        let filtered = filter_integrations(&all, Some(&allowed));
        assert!(filtered.is_empty());
    }

    fn connected_integration(
        toolkit: &str,
    ) -> crate::openhuman::agent::prompts::ConnectedIntegration {
        crate::openhuman::agent::prompts::ConnectedIntegration {
            toolkit: toolkit.to_string(),
            description: String::new(),
            tools: Vec::new(),
            gated_tools: Vec::new(),
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        }
    }

    #[test]
    fn filter_connected_integrations_by_profile_allowlist() {
        // The HasToolkit impl lets the per-profile connector gate reuse
        // filter_integrations on the real ConnectedIntegration type.
        let all = vec![
            connected_integration("gmail"),
            connected_integration("slack"),
            connected_integration("notion"),
        ];
        assert_eq!(filter_integrations(&all, None).len(), 3);
        let allow = vec!["gmail".to_string(), "notion".to_string()];
        let filtered = filter_integrations(&all, Some(&allow));
        let kept: Vec<&str> = filtered.iter().map(|c| c.toolkit_name()).collect();
        assert_eq!(kept, vec!["gmail", "notion"]);
    }
}
