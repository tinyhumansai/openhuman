//! Personality-scoped path resolution and context for multi-agent sessions.

use std::path::{Component, Path};

use crate::openhuman::agent::profiles::AgentProfile;

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
/// Resolution order:
/// 1. `soul_md_path` — read the file at that relative path under workspace.
/// 2. `soul_md` — inline content from the profile.
/// 3. `None` — caller falls back to the workspace root `SOUL.md`.
pub fn resolve_personality_soul(workspace_dir: &Path, profile: &AgentProfile) -> Option<String> {
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
        let memory_suffix = profile.memory_dir_suffix.clone().unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::profiles::AgentProfile;
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
            memory_dir_suffix: None,
            is_master: false,
            sort_order: None,
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
}
