//! Persistent user-selectable agent profiles.
//!
//! Profiles let the UI choose a primary agent persona plus runtime defaults
//! without editing built-in agent TOML. The state is stored under
//! `<workspace>/agent_profiles.json` and merged with built-in profiles on load
//! so new releases can add defaults without overwriting user-created profiles.

use crate::openhuman::context::prompt::{PromptContext, PromptSection};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const PROFILE_FILE: &str = "agent_profiles.json";
pub const DEFAULT_PROFILE_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt_suffix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub built_in: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfilesState {
    pub active_profile_id: String,
    pub profiles: Vec<AgentProfile>,
}

#[derive(Debug, Clone)]
pub struct AgentProfileStore {
    workspace_dir: PathBuf,
}

impl AgentProfileStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

    pub fn load(&self) -> Result<AgentProfilesState, String> {
        let path = self.path();
        tracing::debug!(path = %path.display(), "[agent:profiles] load entry");
        let state = if path.exists() {
            let mut buf = String::new();
            fs::File::open(&path)
                .map_err(|e| {
                    tracing::debug!(
                        path = %path.display(),
                        error = %e,
                        "[agent:profiles] load open_error"
                    );
                    format!("open agent profiles {}: {e}", path.display())
                })?
                .read_to_string(&mut buf)
                .map_err(|e| {
                    tracing::debug!(
                        path = %path.display(),
                        error = %e,
                        "[agent:profiles] load read_error"
                    );
                    format!("read agent profiles {}: {e}", path.display())
                })?;
            serde_json::from_str::<AgentProfilesState>(&buf).map_err(|e| {
                tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "[agent:profiles] load parse_error"
                );
                format!("parse agent profiles {}: {e}", path.display())
            })?
        } else {
            tracing::debug!(path = %path.display(), "[agent:profiles] load default_state");
            AgentProfilesState::default()
        };
        let state = normalise_state(state);
        tracing::debug!(
            path = %path.display(),
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[agent:profiles] load ok"
        );
        Ok(state)
    }

    pub fn save(&self, state: AgentProfilesState) -> Result<AgentProfilesState, String> {
        tracing::debug!(
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[agent:profiles] save entry"
        );
        let state = normalise_state(state);
        let path = self.path();
        let parent = path.parent().ok_or_else(|| {
            tracing::debug!(
                path = %path.display(),
                "[agent:profiles] save invalid_path"
            );
            format!("invalid agent profiles path {}", path.display())
        })?;
        fs::create_dir_all(parent).map_err(|e| {
            tracing::debug!(
                path = %path.display(),
                parent = %parent.display(),
                error = %e,
                "[agent:profiles] save create_dir_error"
            );
            format!("create agent profiles dir {}: {e}", parent.display())
        })?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
            tracing::debug!(
                parent = %parent.display(),
                error = %e,
                "[agent:profiles] save tempfile_error"
            );
            format!(
                "create agent profiles tempfile in {}: {e}",
                parent.display()
            )
        })?;
        let bytes = serde_json::to_vec_pretty(&state).map_err(|e| {
            tracing::debug!(error = %e, "[agent:profiles] save serialize_error");
            format!("serialize agent profiles: {e}")
        })?;
        tmp.write_all(&bytes).map_err(|e| {
            tracing::debug!(error = %e, "[agent:profiles] save write_error");
            format!("write agent profiles tempfile: {e}")
        })?;
        tmp.as_file().sync_all().map_err(|e| {
            tracing::debug!(error = %e, "[agent:profiles] save fsync_error");
            format!("fsync agent profiles tempfile: {e}")
        })?;
        tmp.persist(&path).map_err(|e| {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "[agent:profiles] save persist_error"
            );
            format!("persist agent profiles {}: {e}", path.display())
        })?;
        tracing::debug!(
            path = %path.display(),
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[agent:profiles] save ok"
        );
        Ok(state)
    }

    pub fn select(&self, profile_id: &str) -> Result<AgentProfilesState, String> {
        let mut state = self.load()?;
        let profile_id = profile_id.trim();
        tracing::debug!(profile_id, "[agent:profiles] select entry");
        if !state.profiles.iter().any(|p| p.id == profile_id) {
            tracing::debug!(profile_id, "[agent:profiles] select not_found");
            return Err(format!("agent profile '{profile_id}' not found"));
        }
        state.active_profile_id = profile_id.to_string();
        tracing::debug!(profile_id, "[agent:profiles] select active_profile_changed");
        self.save(state)
    }

    pub fn upsert(&self, profile: AgentProfile) -> Result<AgentProfilesState, String> {
        let mut state = self.load()?;
        let profile = normalise_profile(profile);
        tracing::debug!(
            profile_id = %profile.id,
            agent_id = %profile.agent_id,
            "[agent:profiles] upsert entry"
        );
        let profile = if profile.id == DEFAULT_PROFILE_ID {
            tracing::debug!("[agent:profiles] upsert built_in_default_merge");
            let mut default = built_in_default_profile();
            default.name = profile.name;
            default.description = profile.description;
            default.model_override = profile.model_override;
            default.temperature = profile.temperature;
            default.system_prompt_suffix = profile.system_prompt_suffix;
            default.allowed_tools = profile.allowed_tools;
            default
        } else {
            AgentProfile {
                built_in: profile.built_in
                    || built_in_profiles()
                        .iter()
                        .any(|builtin| builtin.id == profile.id),
                ..profile
            }
        };

        if let Some(existing) = state.profiles.iter_mut().find(|p| p.id == profile.id) {
            tracing::debug!(profile_id = %profile.id, "[agent:profiles] upsert replace_existing");
            *existing = profile;
        } else {
            tracing::debug!(profile_id = %profile.id, "[agent:profiles] upsert insert_new");
            state.profiles.push(profile);
        }
        self.save(state)
    }

    pub fn delete(&self, profile_id: &str) -> Result<AgentProfilesState, String> {
        let profile_id = profile_id.trim();
        tracing::debug!(profile_id, "[agent:profiles] delete entry");
        if built_in_profiles()
            .iter()
            .any(|profile| profile.id == profile_id)
        {
            tracing::debug!(profile_id, "[agent:profiles] delete built_in_rejected");
            return Err(format!(
                "built-in agent profile '{profile_id}' cannot be deleted"
            ));
        }
        let mut state = self.load()?;
        let before = state.profiles.len();
        state.profiles.retain(|p| p.id != profile_id);
        if state.profiles.len() == before {
            tracing::debug!(profile_id, "[agent:profiles] delete not_found");
            return Err(format!("agent profile '{profile_id}' not found"));
        }
        if state.active_profile_id == profile_id {
            state.active_profile_id = DEFAULT_PROFILE_ID.to_string();
            tracing::debug!(
                profile_id,
                "[agent:profiles] delete active_profile_fallback"
            );
        }
        tracing::debug!(
            profile_id,
            profile_count = state.profiles.len(),
            "[agent:profiles] delete removed"
        );
        self.save(state)
    }

    pub fn resolve(
        &self,
        requested_profile_id: Option<&str>,
    ) -> Result<(AgentProfilesState, AgentProfile), String> {
        let state = self.load()?;
        let requested = requested_profile_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or(state.active_profile_id.as_str());
        tracing::debug!(
            requested_profile_id = requested,
            "[agent:profiles] resolve entry"
        );
        let profile = state
            .profiles
            .iter()
            .find(|profile| profile.id == requested)
            .or_else(|| {
                state
                    .profiles
                    .iter()
                    .find(|profile| profile.id == DEFAULT_PROFILE_ID)
            })
            .cloned()
            .unwrap_or_else(built_in_default_profile);
        tracing::debug!(
            requested_profile_id = requested,
            resolved_profile_id = %profile.id,
            agent_id = %profile.agent_id,
            "[agent:profiles] resolve ok"
        );
        Ok((state, profile))
    }

    fn path(&self) -> PathBuf {
        self.workspace_dir.join(PROFILE_FILE)
    }
}

pub fn load_profiles(workspace_dir: &Path) -> Result<AgentProfilesState, String> {
    AgentProfileStore::new(workspace_dir.to_path_buf()).load()
}

pub fn built_in_profiles() -> Vec<AgentProfile> {
    vec![
        built_in_default_profile(),
        AgentProfile {
            id: "research".to_string(),
            name: "Research".to_string(),
            description: "Source-grounded research with web and memory tools.".to_string(),
            agent_id: "researcher".to_string(),
            model_override: Some("agentic-v1".to_string()),
            temperature: Some(0.2),
            system_prompt_suffix: Some(
                "Prioritize source-grounded findings, quote evidence sparingly, and separate facts from inference."
                    .to_string(),
            ),
            allowed_tools: None,
            built_in: true,
        },
        AgentProfile {
            id: "planner".to_string(),
            name: "Planner".to_string(),
            description: "Breaks ambiguous work into ordered task plans.".to_string(),
            agent_id: "planner".to_string(),
            model_override: Some("agentic-v1".to_string()),
            temperature: Some(0.3),
            system_prompt_suffix: Some(
                "Favor explicit task decomposition, dependencies, risks, and concrete next actions."
                    .to_string(),
            ),
            allowed_tools: None,
            built_in: true,
        },
        AgentProfile {
            id: "review".to_string(),
            name: "Review".to_string(),
            description: "Critical review mode for bugs, regressions, and missing tests.".to_string(),
            agent_id: "critic".to_string(),
            model_override: Some("agentic-v1".to_string()),
            temperature: Some(0.1),
            system_prompt_suffix: Some(
                "Lead with concrete findings, cite files or evidence, and avoid broad rewrites unless required."
                    .to_string(),
            ),
            allowed_tools: None,
            built_in: true,
        },
    ]
}

pub fn profile_signature(profile: &AgentProfile) -> String {
    serde_json::to_string(profile).unwrap_or_else(|_| profile.id.clone())
}

pub struct AgentProfilePromptSection {
    body: String,
}

impl AgentProfilePromptSection {
    pub fn new(body: String) -> Self {
        Self { body }
    }
}

impl PromptSection for AgentProfilePromptSection {
    fn name(&self) -> &str {
        "agent_profile"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        if self.body.trim().is_empty() {
            return Ok(String::new());
        }
        Ok(format!("## Agent profile\n\n{}", self.body.trim()))
    }
}

fn built_in_default_profile() -> AgentProfile {
    AgentProfile {
        id: DEFAULT_PROFILE_ID.to_string(),
        name: "Default".to_string(),
        description: "The standard OpenHuman orchestrator.".to_string(),
        agent_id: "orchestrator".to_string(),
        model_override: None,
        temperature: None,
        system_prompt_suffix: None,
        allowed_tools: None,
        built_in: true,
    }
}

fn normalise_state(state: AgentProfilesState) -> AgentProfilesState {
    tracing::trace!(
        active_profile_id = %state.active_profile_id,
        profile_count = state.profiles.len(),
        "[agent:profiles] normalise_state entry"
    );
    let mut by_id: BTreeMap<String, AgentProfile> = built_in_profiles()
        .into_iter()
        .map(|profile| (profile.id.clone(), profile))
        .collect();

    for profile in state.profiles {
        let profile = normalise_profile(profile);
        if profile.id.is_empty() {
            continue;
        }
        by_id.insert(profile.id.clone(), profile);
    }

    let mut profiles: Vec<AgentProfile> = by_id.into_values().collect();
    profiles.sort_by(|a, b| {
        let rank = |id: &str| match id {
            DEFAULT_PROFILE_ID => 0,
            "research" => 1,
            "planner" => 2,
            "review" => 3,
            _ => 10,
        };
        rank(&a.id)
            .cmp(&rank(&b.id))
            .then_with(|| a.name.cmp(&b.name))
    });

    let active_profile_id = state.active_profile_id.trim().to_string();
    let active_profile_id = if profiles.iter().any(|p| p.id == active_profile_id) {
        active_profile_id
    } else {
        DEFAULT_PROFILE_ID.to_string()
    };

    AgentProfilesState {
        active_profile_id,
        profiles,
    }
}

fn normalise_profile(mut profile: AgentProfile) -> AgentProfile {
    profile.id = slugify_profile_id(&profile.id);
    if profile.id.is_empty() {
        profile.id = slugify_profile_id(&profile.name);
    }
    profile.name = profile.name.trim().to_string();
    if profile.name.is_empty() {
        profile.name = profile.id.clone();
    }
    profile.description = profile.description.trim().to_string();
    profile.agent_id = profile.agent_id.trim().to_string();
    if profile.agent_id.is_empty() {
        profile.agent_id = "orchestrator".to_string();
    }
    profile.model_override = profile
        .model_override
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    profile.system_prompt_suffix = profile
        .system_prompt_suffix
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    profile.allowed_tools = profile.allowed_tools.map(|tools| {
        tools
            .into_iter()
            .map(|tool| tool.trim().to_string())
            .filter(|tool| !tool.is_empty())
            .collect::<Vec<_>>()
    });
    if matches!(profile.allowed_tools.as_ref(), Some(tools) if tools.is_empty()) {
        profile.allowed_tools = None;
    }
    profile
}

fn slugify_profile_id(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for c in input.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('-');
            last_was_sep = true;
        }
    }
    out.trim_matches('-').to_string()
}

impl Default for AgentProfilesState {
    fn default() -> Self {
        Self {
            active_profile_id: DEFAULT_PROFILE_ID.to_string(),
            profiles: built_in_profiles(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, PromptContext, ToolCallFormat};
    use std::collections::HashSet;
    use tempfile::tempdir;

    #[test]
    fn profile_store_roundtrips_active_profile_and_custom_entries() {
        let dir = tempdir().expect("tempdir");
        let store = AgentProfileStore::new(dir.path().to_path_buf());
        let state = store
            .upsert(AgentProfile {
                id: " Custom Profile ".into(),
                name: " Custom Profile ".into(),
                description: "  My custom profile ".into(),
                agent_id: " planner ".into(),
                model_override: Some(" agentic-v1 ".into()),
                temperature: Some(0.25),
                system_prompt_suffix: Some("  Be brief. ".into()),
                allowed_tools: Some(vec![" todo ".into(), "".into()]),
                built_in: false,
            })
            .expect("upsert");
        assert!(state.profiles.iter().any(|p| p.id == "custom-profile"));

        let selected = store.select("custom-profile").expect("select");
        assert_eq!(selected.active_profile_id, "custom-profile");

        let loaded = store.load().expect("load");
        let custom = loaded
            .profiles
            .iter()
            .find(|profile| profile.id == "custom-profile")
            .expect("custom profile");
        assert_eq!(custom.agent_id, "planner");
        assert_eq!(
            custom.allowed_tools.as_deref(),
            Some(vec!["todo".to_string()].as_slice())
        );

        let resolved = store.resolve(Some("custom-profile")).expect("resolve").1;
        assert_eq!(resolved.id, "custom-profile");
    }

    #[test]
    fn built_in_profiles_are_merged_when_file_is_missing() {
        let dir = tempdir().expect("tempdir");
        let store = AgentProfileStore::new(dir.path().to_path_buf());
        let loaded = store.load().expect("load");
        let ids: Vec<&str> = loaded.profiles.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&DEFAULT_PROFILE_ID));
        assert!(ids.contains(&"research"));
        assert_eq!(loaded.active_profile_id, DEFAULT_PROFILE_ID);
    }

    #[test]
    fn load_profiles_helper_reads_defaults() {
        let dir = tempdir().expect("tempdir");
        let loaded = load_profiles(dir.path()).expect("load profiles");
        assert!(loaded
            .profiles
            .iter()
            .any(|profile| profile.id == DEFAULT_PROFILE_ID));
    }

    #[test]
    fn normalise_state_falls_back_to_default_active_profile() {
        let state = normalise_state(AgentProfilesState {
            active_profile_id: "missing".into(),
            profiles: vec![AgentProfile {
                id: "  ".into(),
                name: "   ".into(),
                description: " ignored ".into(),
                agent_id: " ".into(),
                model_override: Some(" ".into()),
                temperature: None,
                system_prompt_suffix: Some(" ".into()),
                allowed_tools: Some(vec![" ".into()]),
                built_in: false,
            }],
        });

        assert_eq!(state.active_profile_id, DEFAULT_PROFILE_ID);
        assert!(!state.profiles.iter().any(|profile| profile.id.is_empty()));
    }

    #[test]
    fn upsert_default_profile_preserves_builtin_default_identity() {
        let dir = tempdir().expect("tempdir");
        let store = AgentProfileStore::new(dir.path().to_path_buf());
        let state = store
            .upsert(AgentProfile {
                id: DEFAULT_PROFILE_ID.into(),
                name: " Default Custom ".into(),
                description: " custom description ".into(),
                agent_id: " planner ".into(),
                model_override: Some(" agentic-v1 ".into()),
                temperature: Some(0.3),
                system_prompt_suffix: Some(" suffix ".into()),
                allowed_tools: Some(vec![" todo ".into()]),
                built_in: false,
            })
            .expect("upsert default");
        let default = state
            .profiles
            .iter()
            .find(|profile| profile.id == DEFAULT_PROFILE_ID)
            .expect("default profile");
        assert!(default.built_in);
        assert_eq!(default.agent_id, "orchestrator");
        assert_eq!(default.name, "Default Custom");
        assert_eq!(default.system_prompt_suffix.as_deref(), Some("suffix"));
    }

    #[test]
    fn select_missing_and_delete_builtin_return_errors() {
        let dir = tempdir().expect("tempdir");
        let store = AgentProfileStore::new(dir.path().to_path_buf());

        let select_err = store.select("missing").expect_err("missing select");
        assert!(select_err.contains("not found"));

        let delete_err = store
            .delete(DEFAULT_PROFILE_ID)
            .expect_err("builtin delete rejected");
        assert!(delete_err.contains("cannot be deleted"));
    }

    #[test]
    fn delete_missing_custom_profile_returns_error() {
        let dir = tempdir().expect("tempdir");
        let store = AgentProfileStore::new(dir.path().to_path_buf());
        let err = store.delete("not-there").expect_err("missing delete");
        assert!(err.contains("not found"));
    }

    #[test]
    fn resolve_uses_active_profile_and_falls_back_to_default() {
        let dir = tempdir().expect("tempdir");
        let store = AgentProfileStore::new(dir.path().to_path_buf());
        store
            .upsert(AgentProfile {
                id: "writer".into(),
                name: "Writer".into(),
                description: String::new(),
                agent_id: "planner".into(),
                model_override: None,
                temperature: None,
                system_prompt_suffix: None,
                allowed_tools: None,
                built_in: false,
            })
            .expect("upsert");
        store.select("writer").expect("select");

        let active = store.resolve(None).expect("resolve active").1;
        assert_eq!(active.id, "writer");
        let fallback = store.resolve(Some("missing")).expect("resolve missing").1;
        assert_eq!(fallback.id, DEFAULT_PROFILE_ID);
    }

    #[test]
    fn profile_signature_and_prompt_section_render_expected_text() {
        let profile = built_in_profiles()
            .into_iter()
            .find(|profile| profile.id == "planner")
            .expect("planner profile");
        let signature = profile_signature(&profile);
        assert!(signature.contains("\"planner\""));

        let section = AgentProfilePromptSection::new("  Be concise.  ".into());
        assert_eq!(section.name(), "agent_profile");
        let visible_tool_names = HashSet::new();
        let ctx = PromptContext {
            workspace_dir: std::path::Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "orchestrator",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &visible_tool_names,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            curated_snapshot: None,
            user_identity: None,
        };
        let rendered = section.build(&ctx).expect("render profile section");
        assert!(rendered.starts_with("## Agent profile"));
        assert!(rendered.contains("Be concise."));

        let empty = AgentProfilePromptSection::new("   ".into());
        assert_eq!(empty.build(&ctx).expect("empty profile section"), "");
    }

    #[test]
    fn deleting_active_custom_profile_falls_back_to_default() {
        let dir = tempdir().expect("tempdir");
        let store = AgentProfileStore::new(dir.path().to_path_buf());
        store
            .upsert(AgentProfile {
                id: "tmp".into(),
                name: "Tmp".into(),
                description: String::new(),
                agent_id: "orchestrator".into(),
                model_override: None,
                temperature: None,
                system_prompt_suffix: None,
                allowed_tools: None,
                built_in: false,
            })
            .expect("upsert");
        store.select("tmp").expect("select");
        let state = store.delete("tmp").expect("delete");
        assert_eq!(state.active_profile_id, DEFAULT_PROFILE_ID);
        assert!(!state.profiles.iter().any(|p| p.id == "tmp"));
    }
}
