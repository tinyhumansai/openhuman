//! Persistence layer for agent profiles.
//!
//! State is stored under `<workspace>/agent_profiles.json` and merged with
//! built-in profiles on load so new releases can add defaults without
//! overwriting user-created profiles.

use super::types::{AgentProfile, AgentProfilesState, DEFAULT_PROFILE_ID};
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const PROFILE_FILE: &str = "agent_profiles.json";

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
        tracing::debug!(path = %path.display(), "[profiles] load entry");
        let state = if path.exists() {
            let mut buf = String::new();
            fs::File::open(&path)
                .map_err(|e| {
                    tracing::debug!(
                        path = %path.display(),
                        error = %e,
                        "[profiles] load open_error"
                    );
                    format!("open agent profiles {}: {e}", path.display())
                })?
                .read_to_string(&mut buf)
                .map_err(|e| {
                    tracing::debug!(
                        path = %path.display(),
                        error = %e,
                        "[profiles] load read_error"
                    );
                    format!("read agent profiles {}: {e}", path.display())
                })?;
            serde_json::from_str::<AgentProfilesState>(&buf).map_err(|e| {
                tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "[profiles] load parse_error"
                );
                format!("parse agent profiles {}: {e}", path.display())
            })?
        } else {
            tracing::debug!(path = %path.display(), "[profiles] load default_state");
            AgentProfilesState::default()
        };
        let state = normalise_state(state);
        tracing::debug!(
            path = %path.display(),
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[profiles] load ok"
        );
        Ok(state)
    }

    pub fn save(&self, state: AgentProfilesState) -> Result<AgentProfilesState, String> {
        tracing::debug!(
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[profiles] save entry"
        );
        let state = normalise_state(state);
        let path = self.path();
        let parent = path.parent().ok_or_else(|| {
            tracing::debug!(
                path = %path.display(),
                "[profiles] save invalid_path"
            );
            format!("invalid agent profiles path {}", path.display())
        })?;
        fs::create_dir_all(parent).map_err(|e| {
            tracing::debug!(
                path = %path.display(),
                parent = %parent.display(),
                error = %e,
                "[profiles] save create_dir_error"
            );
            format!("create agent profiles dir {}: {e}", parent.display())
        })?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
            tracing::debug!(
                parent = %parent.display(),
                error = %e,
                "[profiles] save tempfile_error"
            );
            format!(
                "create agent profiles tempfile in {}: {e}",
                parent.display()
            )
        })?;
        let bytes = serde_json::to_vec_pretty(&state).map_err(|e| {
            tracing::debug!(error = %e, "[profiles] save serialize_error");
            format!("serialize agent profiles: {e}")
        })?;
        tmp.write_all(&bytes).map_err(|e| {
            tracing::debug!(error = %e, "[profiles] save write_error");
            format!("write agent profiles tempfile: {e}")
        })?;
        tmp.as_file().sync_all().map_err(|e| {
            tracing::debug!(error = %e, "[profiles] save fsync_error");
            format!("fsync agent profiles tempfile: {e}")
        })?;
        tmp.persist(&path).map_err(|e| {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "[profiles] save persist_error"
            );
            format!("persist agent profiles {}: {e}", path.display())
        })?;
        tracing::debug!(
            path = %path.display(),
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[profiles] save ok"
        );
        Ok(state)
    }

    pub fn select(&self, profile_id: &str) -> Result<AgentProfilesState, String> {
        let mut state = self.load()?;
        let profile_id = profile_id.trim();
        tracing::debug!(profile_id, "[profiles] select entry");
        if !state.profiles.iter().any(|p| p.id == profile_id) {
            tracing::debug!(profile_id, "[profiles] select not_found");
            return Err(format!("agent profile '{profile_id}' not found"));
        }
        state.active_profile_id = profile_id.to_string();
        tracing::debug!(profile_id, "[profiles] select active_profile_changed");
        self.save(state)
    }

    pub fn upsert(&self, profile: AgentProfile) -> Result<AgentProfilesState, String> {
        let mut state = self.load()?;
        let profile = normalise_profile(profile);
        super::home::validate_profile_id(&profile.id)?;
        tracing::debug!(
            profile_id = %profile.id,
            agent_id = %profile.agent_id,
            "[profiles] upsert entry"
        );
        let profile = if profile.id == DEFAULT_PROFILE_ID {
            tracing::debug!("[profiles] upsert built_in_default_merge");
            let mut default = built_in_default_profile();
            default.name = profile.name;
            default.description = profile.description;
            default.model_override = profile.model_override;
            default.temperature = profile.temperature;
            default.system_prompt_suffix = profile.system_prompt_suffix;
            default.allowed_tools = profile.allowed_tools;
            default.avatar_url = profile.avatar_url;
            default.voice_id = profile.voice_id;
            default.soul_md = profile.soul_md;
            default.soul_md_path = profile.soul_md_path;
            default.composio_integrations = profile.composio_integrations;
            default.memory_sources = profile.memory_sources;
            default.include_agent_conversations = profile.include_agent_conversations;
            default.allowed_skills = profile.allowed_skills;
            default.allowed_mcp_servers = profile.allowed_mcp_servers;
            default.dedicated_memory = profile.dedicated_memory;
            default.dedicated_workspace = profile.dedicated_workspace;
            // memory_dir_suffix stays as built-in default (don't let user override the default's suffix)
            default.sort_order = profile.sort_order;
            default
        } else {
            AgentProfile {
                built_in: profile.built_in
                    || built_in_profiles()
                        .iter()
                        .any(|builtin| builtin.id == profile.id),
                is_master: false, // only DEFAULT_PROFILE_ID may be master
                ..profile
            }
        };

        let profile = if profile.id != DEFAULT_PROFILE_ID && profile.memory_dir_suffix.is_none() {
            // Re-upsert of an existing profile without a suffix → reuse the stored
            // suffix so its memory directory doesn't migrate (and silently orphan
            // its database).
            if let Some(existing) = state.profiles.iter().find(|p| p.id == profile.id) {
                if let Some(ref existing_suffix) = existing.memory_dir_suffix {
                    AgentProfile {
                        memory_dir_suffix: Some(existing_suffix.clone()),
                        ..profile
                    }
                } else {
                    // Pre-personality profile getting its first suffix assignment.
                    let existing_suffixes: std::collections::HashSet<String> = state
                        .profiles
                        .iter()
                        .filter(|p| p.id != profile.id)
                        .filter_map(|p| p.memory_dir_suffix.clone())
                        .filter(|s| !s.is_empty())
                        .collect();
                    AgentProfile {
                        memory_dir_suffix: Some(next_available_suffix(&existing_suffixes)),
                        ..profile
                    }
                }
            } else {
                // New non-default profile: assign the lowest unused suffix.
                let existing_suffixes: std::collections::HashSet<String> = state
                    .profiles
                    .iter()
                    .filter_map(|p| p.memory_dir_suffix.clone())
                    .filter(|s| !s.is_empty())
                    .collect();
                AgentProfile {
                    memory_dir_suffix: Some(next_available_suffix(&existing_suffixes)),
                    ..profile
                }
            }
        } else {
            profile
        };

        if let Some(existing) = state.profiles.iter_mut().find(|p| p.id == profile.id) {
            tracing::debug!(profile_id = %profile.id, "[profiles] upsert replace_existing");
            *existing = profile;
        } else {
            tracing::debug!(profile_id = %profile.id, "[profiles] upsert insert_new");
            state.profiles.push(profile);
        }
        self.save(state)
    }

    pub fn delete(&self, profile_id: &str) -> Result<AgentProfilesState, String> {
        let profile_id = profile_id.trim();
        tracing::debug!(profile_id, "[profiles] delete entry");
        if built_in_profiles()
            .iter()
            .any(|profile| profile.id == profile_id)
        {
            tracing::debug!(profile_id, "[profiles] delete built_in_rejected");
            return Err(format!(
                "built-in agent profile '{profile_id}' cannot be deleted"
            ));
        }
        let mut state = self.load()?;
        let before = state.profiles.len();
        state.profiles.retain(|p| p.id != profile_id);
        if state.profiles.len() == before {
            tracing::debug!(profile_id, "[profiles] delete not_found");
            return Err(format!("agent profile '{profile_id}' not found"));
        }
        if state.active_profile_id == profile_id {
            state.active_profile_id = DEFAULT_PROFILE_ID.to_string();
            tracing::debug!(profile_id, "[profiles] delete active_profile_fallback");
        }
        tracing::debug!(
            profile_id,
            profile_count = state.profiles.len(),
            "[profiles] delete removed"
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
        tracing::debug!(requested_profile_id = requested, "[profiles] resolve entry");
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
            "[profiles] resolve ok"
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
            id: "reasoning".to_string(),
            name: "Reasoning".to_string(),
            description: "Deep reasoning mode with extended thinking.".to_string(),
            agent_id: "orchestrator".to_string(),
            model_override: Some("hint:reasoning".to_string()),
            temperature: None,
            system_prompt_suffix: None,
            allowed_tools: None,
            built_in: true,
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
        },
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
        },
    ]
}

pub(crate) fn built_in_default_profile() -> AgentProfile {
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
        avatar_url: None,
        voice_id: None,
        soul_md: None,
        soul_md_path: None,
        composio_integrations: None,
        memory_sources: None,
        include_agent_conversations: true,
        allowed_skills: None,
        allowed_mcp_servers: None,
        memory_dir_suffix: Some("".into()),
        is_master: true,
        sort_order: None,
        dedicated_memory: false,
        dedicated_workspace: false,
    }
}

fn normalise_state(state: AgentProfilesState) -> AgentProfilesState {
    tracing::trace!(
        active_profile_id = %state.active_profile_id,
        profile_count = state.profiles.len(),
        "[profiles] normalise_state entry"
    );
    let mut by_id: BTreeMap<String, AgentProfile> = built_in_profiles()
        .into_iter()
        .map(|profile| (profile.id.clone(), profile))
        .collect();
    // `by_id` currently holds exactly the built-in profile ids — capture them so
    // the overlay loop below can recognise a persisted override of a built-in.
    let built_in_ids: std::collections::HashSet<String> = by_id.keys().cloned().collect();

    for profile in state.profiles {
        let mut profile = normalise_profile(profile);
        if profile.id.is_empty() {
            continue;
        }
        if profile.id == DEFAULT_PROFILE_ID {
            profile.is_master = true;
            profile.memory_dir_suffix = Some(String::new());
        } else if built_in_ids.contains(&profile.id) && !profile.dedicated_memory {
            tracing::debug!(
                profile_id = %profile.id,
                had_suffix = profile.memory_dir_suffix.is_some(),
                "[profiles] normalise pinning built-in profile to shared memory/session_raw subtree"
            );
            // #5351: the built-in helper profiles (reasoning/research/planner/
            // review) ship `dedicated_memory: false` and are meant to SHARE the
            // default memory + `session_raw` subtree. An earlier `upsert` path
            // (store.rs `upsert`) wrongly stamped them with a numeric
            // `memory_dir_suffix` ("-1") — isolating their transcripts + memory
            // recall into `session_raw-1/` / `memory-1/`, which dropped all prior
            // context when the user flipped the Quick/Reasoning toggle mid-thread.
            // Pin them back to the shared subtree here (mirroring how `default` is
            // pinned above, the canonical normalization point that runs on every
            // load AND save), so both a fresh mis-assignment and an
            // already-persisted stale suffix are healed. A user who genuinely
            // wants isolated memory sets `dedicated_memory` (honoured by the guard
            // above) or creates a custom (non-built-in) profile — neither is
            // touched here.
            profile.memory_dir_suffix = None;
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

/// Trim + drop empty entries from an optional string-list allowlist; an empty
/// result normalises to `None` (the "all / unrestricted" sentinel).
fn normalise_allowlist(list: Option<Vec<String>>) -> Option<Vec<String>> {
    let cleaned = list.map(|items| {
        items
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
    });
    match cleaned {
        Some(items) if items.is_empty() => None,
        other => other,
    }
}

fn normalise_profile(mut profile: AgentProfile) -> AgentProfile {
    profile.id = normalise_profile_id(&profile.id, &profile.name);
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
    profile.allowed_tools = normalise_allowlist(profile.allowed_tools);
    profile.avatar_url = profile
        .avatar_url
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    profile.voice_id = profile
        .voice_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    profile.soul_md = profile
        .soul_md
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    profile.soul_md_path = profile
        .soul_md_path
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    profile.composio_integrations = normalise_allowlist(profile.composio_integrations);
    profile.memory_sources = normalise_allowlist(profile.memory_sources);
    profile.allowed_skills = normalise_allowlist(profile.allowed_skills);
    profile.allowed_mcp_servers = normalise_allowlist(profile.allowed_mcp_servers);
    // Note: `Some("")` is the sentinel used exclusively by the default profile
    // to indicate the legacy `memory/` directory (no suffix). `normalise_state`
    // re-applies it after the filter below, so any `Some("")` on a non-default
    // profile is silently dropped to `None` here, causing it to receive the
    // next available numbered suffix on the following `upsert` path.
    profile.memory_dir_suffix = profile
        .memory_dir_suffix
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    profile
}

/// Return the lowest available numbered suffix (`"-1"`, `"-2"`, …) not present
/// in `existing`. Used during `upsert` to auto-assign a unique memory directory
/// suffix to a new non-default personality profile.
fn next_available_suffix(existing: &std::collections::HashSet<String>) -> String {
    let mut n = 1u32;
    loop {
        let candidate = format!("-{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Normalise a raw profile id into its persisted slug form, mirroring the
/// transformation `normalise_profile` applies on upsert. Exposed so callers
/// (e.g. `ops::upsert`) can locate the persisted profile by its stored id after
/// the store has normalised it.
///
/// Takes the name as well as the id because the rule needs both: an id that
/// slugifies to nothing falls back to the name, and a name that slugifies to
/// nothing falls back to a digest of it. A caller that mirrors only the first
/// step looks up an id the store never wrote.
pub(crate) fn normalise_profile_id(id: &str, name: &str) -> String {
    let from_id = slugify_profile_id(id);
    if !from_id.is_empty() {
        return from_id;
    }
    let from_name = slugify_profile_id(name);
    if !from_name.is_empty() {
        return from_name;
    }
    profile_id_from_name_digest(name)
}

/// Profile id for a name that slugifies to nothing.
///
/// `slugify_profile_id` keeps only ASCII alphanumerics, so a name written
/// entirely outside that range - Japanese, Chinese, Greek, Cyrillic, Arabic -
/// reduces to the empty string and `validate_profile_id` rejects the upsert
/// with "profile id must not be empty". The user supplied a name; the error
/// blames an id they never typed.
///
/// Derive one from the name instead. A digest keeps it deterministic, so
/// saving the same profile twice updates it rather than creating a second one.
///
/// 16 bytes, not 4: at 32 bits the birthday bound is ~65k names, and upsert
/// replaces by id, so a collision silently overwrites someone's profile. The
/// resulting 40-character id stays under the 64-character cap in
/// `validate_profile_id`.
fn profile_id_from_name_digest(name: &str) -> String {
    use sha2::{Digest, Sha256};

    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let digest = Sha256::digest(trimmed.as_bytes());
    let short: String = digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("profile-{short}")
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
#[path = "store_tests.rs"]
mod tests;
