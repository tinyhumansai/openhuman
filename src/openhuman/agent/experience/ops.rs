use serde::{Deserialize, Serialize};

use crate::openhuman::agent::experience::store::{
    retrieve_across_stores, AgentExperienceStore, ExperienceQuery,
};
use crate::openhuman::agent::experience::types::{AgentExperience, ExperienceHit};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct CaptureParams {
    pub experience: AgentExperience,
}

#[derive(Debug, Deserialize, Default)]
pub struct RetrieveParams {
    pub query: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    /// Profile partition filter (1c). `None` (omitted) recalls the whole pool;
    /// `Some(P)` recalls records stamped `P` plus unstamped legacy records.
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub max_hits: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListParams {
    /// Profile partition filter (1c), same semantics as `RetrieveParams`.
    #[serde(default)]
    pub profile_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DismissParams {
    pub id: String,
    #[serde(default)]
    pub profile_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DismissResult {
    pub id: String,
    pub dismissed: bool,
}

fn profile_memory_subdir(
    workspace_dir: &std::path::Path,
    profile_id: Option<&str>,
) -> Result<String, String> {
    let Some(profile_id) = profile_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok("memory".to_string());
    };
    let state = crate::openhuman::agent::profiles::load_profiles(workspace_dir)?;
    let profile = state
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("agent profile '{profile_id}' not found"))?;
    let suffix = crate::openhuman::agent::profiles::effective_memory_suffix(profile);
    Ok(crate::openhuman::agent::profiles::memory_subdir_for_suffix(
        &suffix,
    ))
}

async fn open_store(profile_id: Option<&str>) -> Result<AgentExperienceStore, String> {
    let profile_id = profile_id.map(str::trim).filter(|id| !id.is_empty());
    if profile_id.is_none() {
        let client = match crate::openhuman::memory::global::client_if_ready() {
            Some(client) => client,
            None => {
                let config = Config::load_or_init()
                    .await
                    .map_err(|e| format!("load config: {e}"))?;
                crate::openhuman::memory::global::init(config.workspace_dir)?
            }
        };
        return Ok(AgentExperienceStore::new(client.memory_handle()));
    }

    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let memory_subdir = profile_memory_subdir(&config.workspace_dir, profile_id)?;

    open_store_in_subdir(&config, &memory_subdir).await
}

async fn open_store_in_subdir(
    config: &Config,
    memory_subdir: &str,
) -> Result<AgentExperienceStore, String> {
    if memory_subdir != "memory" {
        let memory = crate::openhuman::memory::store::UnifiedMemory::new_with_memory_dir(
            &config.workspace_dir,
            memory_subdir,
            crate::openhuman::inference::embeddings::default_embedding_provider(),
            config.memory.sqlite_open_timeout_secs,
        )
        .map_err(|e| format!("open agent experience store '{memory_subdir}': {e:#}"))?;
        return Ok(AgentExperienceStore::new(Arc::new(memory)));
    }

    let client = match crate::openhuman::memory::global::client_if_ready() {
        Some(client) => client,
        None => crate::openhuman::memory::global::init(config.workspace_dir.clone())?,
    };
    Ok(AgentExperienceStore::new(client.memory_handle()))
}

fn query_memory_subdirs(
    workspace_dir: &std::path::Path,
    profile_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let state = crate::openhuman::agent::profiles::load_profiles(workspace_dir)?;
    let mut subdirs = BTreeSet::from(["memory".to_string()]);
    let profile_id = profile_id.map(str::trim).filter(|id| !id.is_empty());

    for profile in &state.profiles {
        if profile_id.is_none_or(|id| profile.id == id) {
            let suffix = crate::openhuman::agent::profiles::effective_memory_suffix(profile);
            subdirs.insert(crate::openhuman::agent::profiles::memory_subdir_for_suffix(
                &suffix,
            ));
        }
    }
    if let Some(profile_id) = profile_id {
        if !state
            .profiles
            .iter()
            .any(|profile| profile.id == profile_id)
        {
            return Err(format!("agent profile '{profile_id}' not found"));
        }
    }
    Ok(subdirs.into_iter().collect())
}

async fn open_query_stores(profile_id: Option<&str>) -> Result<Vec<AgentExperienceStore>, String> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let subdirs = query_memory_subdirs(&config.workspace_dir, profile_id)?;
    let mut stores = Vec::with_capacity(subdirs.len());
    for subdir in subdirs {
        stores.push(open_store_in_subdir(&config, &subdir).await?);
    }
    Ok(stores)
}

pub async fn capture(params: CaptureParams) -> Result<RpcOutcome<AgentExperience>, String> {
    let store = open_store(params.experience.profile_id.as_deref()).await?;
    let stored = store.put(params.experience).await?;
    Ok(RpcOutcome::single_log(stored, "agent experience captured"))
}

pub async fn retrieve(params: RetrieveParams) -> Result<RpcOutcome<Vec<ExperienceHit>>, String> {
    let stores = open_query_stores(params.profile_id.as_deref()).await?;
    let max_hits = params.max_hits.unwrap_or(5);
    let query = ExperienceQuery {
        query: params.query,
        tools: params.tools,
        tags: params.tags,
        agent_id: params.agent_id,
        entrypoint: params.entrypoint,
        profile_id: params.profile_id,
        max_hits,
    };
    let hits = retrieve_across_stores(&stores, query).await?;
    Ok(RpcOutcome::single_log(hits, "agent experiences retrieved"))
}

pub async fn list(params: ListParams) -> Result<RpcOutcome<Vec<AgentExperience>>, String> {
    let stores = open_query_stores(params.profile_id.as_deref()).await?;
    let mut by_id: BTreeMap<String, AgentExperience> = BTreeMap::new();
    for store in stores {
        for experience in store.list_for_profile(params.profile_id.as_deref()).await? {
            let id = experience.id.clone();
            match by_id.get(&id) {
                Some(existing) if existing.updated_at_ms >= experience.updated_at_ms => {}
                _ => {
                    by_id.insert(id, experience);
                }
            }
        }
    }
    let mut experiences: Vec<_> = by_id.into_values().collect();
    experiences.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(RpcOutcome::single_log(
        experiences,
        "agent experiences listed",
    ))
}

pub async fn dismiss(params: DismissParams) -> Result<RpcOutcome<DismissResult>, String> {
    let stores = open_query_stores(params.profile_id.as_deref()).await?;
    let mut dismissed = false;
    for store in stores {
        dismissed |= store
            .dismiss_for_profile(&params.id, params.profile_id.as_deref())
            .await?;
    }
    Ok(RpcOutcome::single_log(
        DismissResult {
            id: params.id,
            dismissed,
        },
        "agent experience dismissed",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_memory_subdir_matches_live_session_derivation() {
        let workspace = tempfile::TempDir::new().unwrap();
        let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
        profile.id = "alice".into();
        profile.name = "Alice".into();
        profile.built_in = false;
        profile.is_master = false;
        profile.dedicated_memory = true;
        crate::openhuman::agent::profiles::store::AgentProfileStore::new(
            workspace.path().to_path_buf(),
        )
        .upsert(profile)
        .expect("seed profile");

        assert_eq!(
            profile_memory_subdir(workspace.path(), Some("alice")).unwrap(),
            "memory-alice"
        );
        assert_eq!(
            profile_memory_subdir(workspace.path(), None).unwrap(),
            "memory"
        );
        assert!(profile_memory_subdir(workspace.path(), Some("missing")).is_err());

        assert_eq!(
            query_memory_subdirs(workspace.path(), Some("alice")).unwrap(),
            vec!["memory".to_string(), "memory-alice".to_string()]
        );
        assert_eq!(
            query_memory_subdirs(workspace.path(), None).unwrap(),
            vec!["memory".to_string(), "memory-alice".to_string()]
        );
    }
}
