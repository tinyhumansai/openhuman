//! What the UI sees of the lifecycle: the recall policy, a preview of the
//! pack a turn would get, the agents with conversations, and the job queue.

use serde::{Deserialize, Serialize};
use tinymemory_tools::{ContextPack, PreviewMode, RecallSettings};

use crate::config::Config;
use tinymemory_api::Result;

use super::jobs;
use tinymemory_tools::queue::{JobRun, QueueState, Selection};

/// `memory_policy_get` / `_set` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyView {
    /// Whether every turn is logged.
    pub log_conversations: bool,
    /// The pack and its budgets.
    pub recall: RecallSettings,
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
    let resolved = tinymemory_tools::layout::identity::Identity::root()
        .resolve(&crate::memory::scope::identity_pins(config));
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
    /// Whether to use a small model call to identify dates in each turn.
    #[serde(default)]
    pub date_hint: Option<bool>,
    /// How long compaction waits for recalled context, in milliseconds.
    #[serde(default)]
    pub compaction_timeout_ms: Option<u64>,
    /// How long queued belief builds wait for newer writes, in seconds.
    /// Must fit within chrono's supported date range when subtracted from now.
    #[serde(default)]
    pub build_delay_secs: Option<u64>,
}

/// Applies `params` to `config`; the caller persists it.
pub fn apply_policy_set(config: &mut Config, params: &PolicySetParams) -> Result<()> {
    // Apply the complete proposed recall state to a copy, then let TinyMemory
    // validate it before changing either recall or conversation settings.
    let mut recall = config.memory.recall.clone();
    if let Some(value) = params.recall_enabled {
        recall.enabled = value;
    }
    if let Some(value) = params.budget_tokens {
        recall.budget_tokens = value;
    }
    if let Some(value) = params.learnings_limit {
        recall.learnings_limit = value;
    }
    if let Some(value) = params.brain_limit {
        recall.brain_limit = value;
    }
    if let Some(value) = params.history_limit {
        recall.history_limit = value;
    }
    if let Some(value) = params.team_limit {
        recall.team_limit = value;
    }
    if let Some(value) = params.build_beliefs_every {
        recall.build_beliefs_every = value;
    }
    if let Some(value) = params.pre_turn_timeout_ms {
        recall.pre_turn_timeout_ms = value;
    }
    if let Some(value) = params.date_hint {
        recall.date_hint = value;
    }
    if let Some(value) = params.compaction_timeout_ms {
        recall.compaction_timeout_ms = value;
    }
    if let Some(value) = params.build_delay_secs {
        recall.build_delay_secs = value;
    }
    recall.validate()?;

    if let Some(value) = params.log_conversations {
        config.memory.conversations.enabled = value;
    }
    config.memory.recall = recall;
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
    identity: &tinymemory_tools::layout::identity::Identity,
    params: PackPreviewParams,
) -> Result<PackPreviewView> {
    let (memory, identity) =
        super::named_agent_memory(config, identity, params.agent_id.as_deref()).await?;
    let preview = memory
        .preview(params.query.as_deref(), params.thread_id.as_deref())
        .await?;
    let mode = match preview.mode {
        PreviewMode::Turn => "turn",
        PreviewMode::Session => "session",
    };
    tracing::debug!(
        mode,
        tokens = preview.pack.tokens,
        "[memory:views] pack preview"
    );
    Ok(PackPreviewView {
        root: identity.root().to_string(),
        agent_id: identity.agent_id,
        mode,
        pack: preview.pack,
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
pub async fn agents_list(
    config: &Config,
    identity: &tinymemory_tools::layout::identity::Identity,
) -> Result<AgentsView> {
    let (memory, identity) = super::current_agent_memory(config, identity).await?;
    let agents = memory
        .agents()
        .await?
        .into_iter()
        .map(|agent| AgentCount {
            agent_id: agent.agent_id,
            turns: agent.turns,
        })
        .collect();
    Ok(AgentsView {
        root: identity.root().to_string(),
        agents,
    })
}

/// `memory_jobs_list` result.
pub type JobsView = QueueState;

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
pub async fn jobs_run(config: &Config, params: JobsRunParams) -> Result<JobsRunView> {
    Ok(JobsRunView {
        runs: jobs::run(
            config,
            params
                .id
                .filter(|id| !id.trim().is_empty())
                .map_or(Selection::All, Selection::One),
        )
        .await?,
    })
}

#[cfg(test)]
#[path = "views_tests.rs"]
mod tests;
