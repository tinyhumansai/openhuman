use serde::{Deserialize, Serialize};

use crate::openhuman::agent_experience::store::{AgentExperienceStore, ExperienceQuery};
use crate::openhuman::agent_experience::types::{AgentExperience, ExperienceHit};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

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
    #[serde(default)]
    pub max_hits: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DismissParams {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct DismissResult {
    pub id: String,
    pub dismissed: bool,
}

async fn open_store() -> Result<AgentExperienceStore, String> {
    let client = match crate::openhuman::memory::global::client_if_ready() {
        Some(client) => client,
        None => {
            let config = Config::load_or_init()
                .await
                .map_err(|e| format!("load config: {e}"))?;
            crate::openhuman::memory::global::init(config.workspace_dir)?
        }
    };
    Ok(AgentExperienceStore::new(client.memory_handle()))
}

pub async fn capture(params: CaptureParams) -> Result<RpcOutcome<AgentExperience>, String> {
    let store = open_store().await?;
    let stored = store.put(params.experience).await?;
    Ok(RpcOutcome::single_log(stored, "agent experience captured"))
}

pub async fn retrieve(params: RetrieveParams) -> Result<RpcOutcome<Vec<ExperienceHit>>, String> {
    let store = open_store().await?;
    let hits = store
        .retrieve(ExperienceQuery {
            query: params.query,
            tools: params.tools,
            tags: params.tags,
            agent_id: params.agent_id,
            entrypoint: params.entrypoint,
            max_hits: params.max_hits.unwrap_or(5),
        })
        .await?;
    Ok(RpcOutcome::single_log(hits, "agent experiences retrieved"))
}

pub async fn list() -> Result<RpcOutcome<Vec<AgentExperience>>, String> {
    let store = open_store().await?;
    let experiences = store.list().await?;
    Ok(RpcOutcome::single_log(
        experiences,
        "agent experiences listed",
    ))
}

pub async fn dismiss(params: DismissParams) -> Result<RpcOutcome<DismissResult>, String> {
    let store = open_store().await?;
    let dismissed = store.dismiss(&params.id).await?;
    Ok(RpcOutcome::single_log(
        DismissResult {
            id: params.id,
            dismissed,
        },
        "agent experience dismissed",
    ))
}
