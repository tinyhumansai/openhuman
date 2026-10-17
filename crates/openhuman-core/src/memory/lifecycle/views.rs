//! What the UI sees of the lifecycle: the recall policy, a preview of the
//! pack a turn would get, the agents with conversations, and the job queue.

use serde::{Deserialize, Serialize};
use tinymemory_api::{ExploreRequest, Facet};
use tinymemory_tools::{ContextPack, SessionStart};

use crate::config::schema::MemoryRecallConfig;
use crate::config::Config;
use crate::memory::error::{MemoryError, MemoryResult};
use crate::memory::scope::{self, MemoryIdentity};

use super::jobs::{self, JobQueue, JobRun, Selection};

/// `memory_policy_get` / `_set` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyView {
    /// Whether every turn is logged.
    pub log_conversations: bool,
    /// The pack and its budgets.
    pub recall: MemoryRecallConfig,
    /// The layout root work outside an agent resolves to.
    pub root: String,
    /// The memory agent id work outside an agent resolves to.
    pub agent_id: String,
    /// Whether a host binding (`[memory] agent_id` / `root`) is in force.
    pub host_bound: bool,
}

/// `memory_policy_get`.
#[must_use]
pub fn policy_view(config: &Config) -> PolicyView {
    let resolved = scope::MemoryIdentity::root().resolve(config);
    PolicyView {
        log_conversations: config.memory.conversations.enabled,
        recall: config.memory.recall.clone(),
        root: resolved.root().to_string(),
        agent_id: resolved.agent_id,
        host_bound: config.memory.agent_id.is_some() || config.memory.root.is_some(),
    }
}

/// `memory_policy_set` params: every field optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicySetParams {
    /// Log every turn.
    #[serde(default)]
    pub log_conversations: Option<bool>,
    /// Inject a pack before every turn.
    #[serde(default)]
    pub recall_enabled: Option<bool>,
    /// The pack's size in tokens (100–16000).
    #[serde(default)]
    pub budget_tokens: Option<u32>,
    /// Learnings and beliefs per pack (0–50).
    #[serde(default)]
    pub learnings_limit: Option<u32>,
    /// Brain documents per pack (0–50).
    #[serde(default)]
    pub brain_limit: Option<u32>,
    /// This agent's turns per pack (0–50).
    #[serde(default)]
    pub history_limit: Option<u32>,
    /// Other agents' turns per pack (0–50).
    #[serde(default)]
    pub team_limit: Option<u32>,
    /// Turns between belief builds; 0 turns them off (0–1000).
    #[serde(default)]
    pub build_beliefs_every: Option<u32>,
    /// How long a turn waits for its pack, in ms (100–30000).
    #[serde(default)]
    pub pre_turn_timeout_ms: Option<u64>,
}

fn within<T: PartialOrd + std::fmt::Display + Copy>(
    name: &str,
    value: Option<T>,
    min: T,
    max: T,
) -> MemoryResult<Option<T>> {
    match value {
        Some(value) if value < min || value > max => Err(MemoryError::invalid(format!(
            "{name} must be between {min} and {max}"
        ))),
        other => Ok(other),
    }
}

/// Applies `params` to `config`; the caller persists it.
pub fn apply_policy_set(config: &mut Config, params: &PolicySetParams) -> MemoryResult<()> {
    let recall = &mut config.memory.recall;
    if let Some(value) = params.log_conversations {
        config.memory.conversations.enabled = value;
    }
    if let Some(value) = params.recall_enabled {
        recall.enabled = value;
    }
    if let Some(value) = within("budget_tokens", params.budget_tokens, 100, 16_000)? {
        recall.budget_tokens = value;
    }
    for (name, value, slot) in [
        (
            "learnings_limit",
            params.learnings_limit,
            &mut recall.learnings_limit,
        ),
        ("brain_limit", params.brain_limit, &mut recall.brain_limit),
        (
            "history_limit",
            params.history_limit,
            &mut recall.history_limit,
        ),
        ("team_limit", params.team_limit, &mut recall.team_limit),
    ] {
        if let Some(value) = within(name, value, 0, 50)? {
            *slot = value;
        }
    }
    if let Some(value) = within("build_beliefs_every", params.build_beliefs_every, 0, 1000)? {
        recall.build_beliefs_every = value;
    }
    if let Some(value) = within(
        "pre_turn_timeout_ms",
        params.pre_turn_timeout_ms,
        100,
        30_000,
    )? {
        recall.pre_turn_timeout_ms = value;
    }
    tracing::info!("[memory:policy] recall policy updated");
    Ok(())
}

/// `memory_pack_preview` params.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackPreviewParams {
    /// What the turn would say; omitted previews a cold session start.
    #[serde(default)]
    pub query: Option<String>,
    /// A thread to resume (session mode).
    #[serde(default)]
    pub thread_id: Option<String>,
    /// The memory agent to preview as; the default agent when omitted.
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// `memory_pack_preview` result: the pack, and what went into it.
#[derive(Debug, Clone, Serialize)]
pub struct PackPreviewView {
    /// The memory agent it was read as.
    pub agent_id: String,
    /// The layout root.
    pub root: String,
    /// `turn` (ranked for a query) or `session` (a session start).
    pub mode: &'static str,
    /// The pack.
    pub pack: ContextPack,
}

/// `memory_pack_preview`: the pack a turn (with a query) or a session start
/// (without one) would be given. Reads only; nothing is logged.
pub async fn pack_preview(
    config: &Config,
    identity: &MemoryIdentity,
    params: PackPreviewParams,
) -> MemoryResult<PackPreviewView> {
    let (memory, identity) =
        super::named_agent_memory(config, identity, params.agent_id.as_deref())?;
    let query = params.query.filter(|query| !query.trim().is_empty());
    let (mode, pack) = match query {
        Some(query) => ("turn", memory.recall(&query).await?),
        None => (
            "session",
            memory
                .start_session(SessionStart {
                    thread_id: params.thread_id.filter(|id| !id.trim().is_empty()),
                    focus: None,
                })
                .await?,
        ),
    };
    tracing::debug!(mode, tokens = pack.tokens, "[memory:views] pack preview");
    Ok(PackPreviewView {
        root: identity.root().to_string(),
        agent_id: identity.agent_id,
        mode,
        pack,
    })
}

/// One agent with conversations in memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentCount {
    /// The memory agent id.
    pub agent_id: String,
    /// Logged turns.
    pub turns: u64,
}

/// `memory_agents_list` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentsView {
    /// The layout root.
    pub root: String,
    /// Every agent with logged turns, most first.
    pub agents: Vec<AgentCount>,
}

/// `memory_agents_list`: the agents with conversations under the root.
pub async fn agents_list(config: &Config, identity: &MemoryIdentity) -> MemoryResult<AgentsView> {
    let (memory, identity) = super::current_agent_memory(config, identity)?;
    let page = memory
        .engine()
        .explore(ExploreRequest {
            facet: Facet::Agent,
            filter: identity.layout.conversations_filter(None),
            limit: 200,
            scan_limit: 20_000,
        })
        .await?;
    let agents = page
        .buckets
        .into_iter()
        .map(|bucket| AgentCount {
            agent_id: bucket.value,
            turns: bucket.count,
        })
        .collect();
    Ok(AgentsView {
        root: identity.root().to_string(),
        agents,
    })
}

/// `memory_jobs_list` result.
pub type JobsView = JobQueue;

/// `memory_jobs_list`.
pub async fn jobs_list(config: &Config) -> JobsView {
    jobs::snapshot(config).await
}

/// `memory_jobs_run` params.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct JobsRunParams {
    /// One job; every pending job when omitted.
    #[serde(default)]
    pub id: Option<String>,
}

/// `memory_jobs_run` result.
#[derive(Debug, Clone, Serialize)]
pub struct JobsRunView {
    /// What ran, in order.
    pub runs: Vec<JobRun>,
}

/// `memory_jobs_run`: runs now, ignoring the build delay.
pub async fn jobs_run(config: &Config, params: JobsRunParams) -> MemoryResult<JobsRunView> {
    let selection = match params.id.filter(|id| !id.trim().is_empty()) {
        Some(id) => Selection::One(id),
        None => Selection::All,
    };
    Ok(JobsRunView {
        runs: jobs::run(config, selection).await?,
    })
}

#[cfg(test)]
#[path = "views_tests.rs"]
mod tests;
