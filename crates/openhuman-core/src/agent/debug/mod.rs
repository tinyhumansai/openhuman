//! Debug helper that renders the exact system prompt a live session
//! would see for a given agent.
//!
//! Instead of re-implementing prompt assembly, this module routes
//! through [`OpenHumanSessionHost::from_config_for_agent`] — the same entry point the
//! Tauri web channel and CLI use — and then calls
//! [`OpenHumanSessionHost::build_system_prompt`] on the constructed session. The
//! output is byte-identical to what the LLM would receive on turn 1 of
//! that agent.
//!
//! Entry points:
//! * [`dump_agent_prompt`] — dump a single agent by id.
//! * [`dump_all_agent_prompts`] — dump every registered agent in one call.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

pub mod dump_writer;
pub mod prompt_size;
pub mod wire;
pub use dump_writer::{write_prompt_dumps, DumpWriteSummary};
pub use wire::render as render_wire_dump;

use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::session_host::OpenHumanSessionHost;
use crate::config::Config;
use tinytools::ToolCategory;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Inputs for [`dump_agent_prompt`].
#[derive(Debug, Clone)]
pub struct DumpPromptOptions {
    /// Target agent id (any id registered in [`AgentDefinitionRegistry`]).
    pub agent_id: String,
    /// Optional override for the workspace directory.
    pub workspace_dir_override: Option<PathBuf>,
    pub config_path_override: Option<PathBuf>,
    /// Optional override for the resolved model name.
    pub model_override: Option<String>,
}

impl DumpPromptOptions {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            workspace_dir_override: None,
            config_path_override: None,
            model_override: None,
        }
    }
}

/// Result of a single prompt dump.
#[derive(Debug, Clone)]
pub struct DumpedPrompt {
    /// Echoed from [`DumpPromptOptions::agent_id`].
    pub agent_id: String,
    /// Always `"session"` — dumps come from the live session path.
    pub mode: &'static str,
    /// Resolved model name.
    pub model: String,
    /// Workspace directory used for identity file injection.
    pub workspace_dir: PathBuf,
    /// The final rendered system prompt — frozen bytes that would be
    /// sent verbatim on every turn of a live session.
    pub text: String,
    /// Every tool the agent can call, in registration order. On the session
    /// path this is the whole callable surface (`all_tool_refs()`), so it can
    /// be wider than [`Self::tool_specs`]; `prompt-size` measures the latter.
    pub tool_names: Vec<String>,
    /// Number of `ToolCategory::Workflow` tools in the dump.
    pub skill_tool_count: usize,
    /// One `{name, description, parameters}` entry per tool schema the
    /// provider receives. On the session path this is the visible set, which
    /// is narrower than [`Self::tool_names`].
    ///
    /// The system prompt is only half of a turn's fixed cost: the tool
    /// schemas ride alongside it in every request, and for an agent with a
    /// few hundred tools they dominate. Dumping the prompt without them
    /// measures the smaller half.
    pub tool_specs: Vec<serde_json::Value>,
}

/// Render and return the system prompt for a single agent via the
/// real [`OpenHumanSessionHost::from_config_for_agent`] construction path.
pub async fn dump_agent_prompt(options: DumpPromptOptions) -> Result<DumpedPrompt> {
    let config = load_dump_config(
        options.workspace_dir_override.clone(),
        options.config_path_override.clone(),
        options.model_override.clone(),
    )
    .await?;

    // Ensure the registry is populated — `from_config_for_agent`
    // errors for any non-orchestrator id when the global registry
    // hasn't been initialised.
    AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .context("initialising AgentDefinitionRegistry for prompt dump")?;

    render_via_session(&config, &options.agent_id).await
}

/// Dump every registered agent's system prompt in one shot.
///
/// The synthetic `fork` archetype is skipped (byte-stable replay, no
/// standalone prompt). Order follows [`AgentDefinitionRegistry::list`].
pub async fn dump_all_agent_prompts(
    workspace_dir_override: Option<PathBuf>,
    config_path_override: Option<PathBuf>,
    model_override: Option<String>,
) -> Result<Vec<DumpedPrompt>> {
    let config =
        load_dump_config(workspace_dir_override, config_path_override, model_override).await?;

    AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .context("initialising AgentDefinitionRegistry for prompt dump")?;

    let registry = AgentDefinitionRegistry::global()
        .ok_or_else(|| anyhow!("AgentDefinitionRegistry missing after init"))?;

    let ids: Vec<String> = registry
        .list()
        .iter()
        .filter(|d| d.id != "fork")
        .map(|d| d.id.clone())
        .collect();

    let mut results = Vec::with_capacity(ids.len());
    for id in ids {
        let dumped = render_via_session(&config, &id)
            .await
            .with_context(|| format!("rendering prompt for agent `{id}`"))?;
        results.push(dumped);
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

async fn load_dump_config(
    workspace_dir_override: Option<PathBuf>,
    config_path_override: Option<PathBuf>,
    model_override: Option<String>,
) -> Result<Config> {
    let mut config = if let Some(path) = config_path_override {
        let workspace = workspace_dir_override
            .as_deref()
            .ok_or_else(|| anyhow!("config path override requires workspace override"))?;
        Config::load_from_config_path(&path, workspace)
            .await
            .context("loading hermetic Config for prompt dump")?
    } else {
        Config::load_or_init()
            .await
            .context("loading Config for prompt dump")?
    };
    config.apply_env_overrides();
    if let Some(override_dir) = workspace_dir_override {
        config.workspace_dir = override_dir;
    }
    std::fs::create_dir_all(&config.workspace_dir).ok();
    if let Some(model) = model_override {
        config.default_model = Some(model);
    }

    Ok(config)
}

/// Build a real [`Agent`] via `from_config_for_agent`, populate live
/// connected integrations, and render the turn-1 system prompt.
async fn render_via_session(config: &Config, agent_id: &str) -> Result<DumpedPrompt> {
    let mut agent = OpenHumanSessionHost::from_config_for_agent(config, agent_id)
        .with_context(|| format!("building session agent for `{agent_id}`"))?;

    // Match turn-1 behaviour: fetch the user's active Composio
    // connections so the rendered prompt mirrors what the LLM actually
    // sees. Best-effort — failures degrade to an empty integration
    // list, same as the live runtime.
    agent.fetch_connected_integrations().await;
    // Mirror turn-1: synthesise `delegate_*` tools for connected
    // Composio toolkits now that we know what's actually authorised.
    agent.refresh_delegation_tools();

    let text = agent
        .build_system_prompt()
        .with_context(|| format!("rendering system prompt for `{agent_id}`"))?;

    Ok(session_dump(&agent, agent_id, text))
}

/// Package a built session agent's rendered prompt and tool surface.
fn session_dump(agent: &OpenHumanSessionHost, agent_id: &str, text: String) -> DumpedPrompt {
    // The whole callable surface, so the dump shows the `delegate_*` tools
    // the refresh above just synthesised alongside the durable registry.
    let tools = agent.all_tool_refs();
    let tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
    let skill_tool_count = tools
        .iter()
        .filter(|t| t.category() == ToolCategory::Workflow)
        .count();
    // Schemas are what the provider is actually sent: the visible,
    // policy-filtered set, not the registry above. Measuring
    // `all_tool_refs()` here (d149ab0f0) reported ~200 tools for every agent.
    let tool_specs = agent
        .visible_tool_specs_arc()
        .iter()
        .map(|spec| serde_json::to_value(spec.as_ref()).unwrap_or_default())
        .collect();

    DumpedPrompt {
        agent_id: agent_id.to_string(),
        mode: "session",
        model: agent.model_name().to_string(),
        workspace_dir: agent.workspace_dir().to_path_buf(),
        text,
        tool_names,
        skill_tool_count,
        tool_specs,
    }
}
