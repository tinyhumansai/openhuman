//! Debug helper that renders the exact system prompt a live session
//! would see for a given agent.
//!
//! Instead of re-implementing prompt assembly, this module routes
//! through [`Agent::from_config_for_agent`] — the same entry point the
//! Tauri web channel, CLI, and `welcome_proactive` all use — and then
//! calls [`Agent::build_system_prompt`] on the constructed session. The
//! output is byte-identical to what the LLM would receive on turn 1 of
//! that agent.
//!
//! Entry points:
//! * [`dump_agent_prompt`] — dump a single agent by id.
//! * [`dump_all_agent_prompts`] — dump every registered agent in one call.
//!
//! `integrations_agent` is special: it is platform-parameterised and
//! has no meaningful prompt without a `toolkit` argument. Callers must
//! supply one (e.g. `"gmail"`, `"notion"`) via
//! [`DumpPromptOptions::toolkit`]; `dump_all_agent_prompts` expands
//! `integrations_agent` into one dump per currently-connected Composio
//! toolkit.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, PromptSource,
};
use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::composio::ComposioActionTool;
use crate::openhuman::config::Config;
use crate::openhuman::context::prompt::{
    LearnedContextData, PromptContext, PromptTool, ToolCallFormat,
};
use crate::openhuman::tools::{Tool, ToolCategory};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Id reserved for the Composio-backed integrations specialist.
const INTEGRATIONS_AGENT_ID: &str = "integrations_agent";

/// Inputs for [`dump_agent_prompt`].
#[derive(Debug, Clone)]
pub struct DumpPromptOptions {
    /// Target agent id (any id registered in [`AgentDefinitionRegistry`]).
    pub agent_id: String,
    /// Composio toolkit to bind this dump to (e.g. `"gmail"`,
    /// `"notion"`). **Required** when `agent_id == "integrations_agent"`
    /// — the integrations specialist has no meaningful prompt without a
    /// toolkit. Must match a currently-connected integration.
    pub toolkit: Option<String>,
    /// Optional override for the workspace directory.
    pub workspace_dir_override: Option<PathBuf>,
    /// Optional override for the resolved model name.
    pub model_override: Option<String>,
}

impl DumpPromptOptions {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            toolkit: None,
            workspace_dir_override: None,
            model_override: None,
        }
    }
}

/// Result of a single prompt dump.
#[derive(Debug, Clone)]
pub struct DumpedPrompt {
    /// Echoed from [`DumpPromptOptions::agent_id`].
    pub agent_id: String,
    /// Composio toolkit this dump was scoped to (set for
    /// `integrations_agent`, `None` for everything else). Lets the CLI
    /// / harness differentiate per-toolkit dumps on disk.
    pub toolkit: Option<String>,
    /// Always `"session"` — dumps come from the live session path.
    pub mode: &'static str,
    /// Resolved model name.
    pub model: String,
    /// Workspace directory used for identity file injection.
    pub workspace_dir: PathBuf,
    /// The final rendered system prompt — frozen bytes that would be
    /// sent verbatim on every turn of a live session.
    pub text: String,
    /// Tool names that made it into the rendered prompt, in order.
    pub tool_names: Vec<String>,
    /// Number of `ToolCategory::Skill` tools in the dump.
    pub skill_tool_count: usize,
}

/// Render and return the system prompt for a single agent via the
/// real [`Agent::from_config_for_agent`] construction path.
pub async fn dump_agent_prompt(options: DumpPromptOptions) -> Result<DumpedPrompt> {
    let config = load_dump_config(
        options.workspace_dir_override.clone(),
        options.model_override.clone(),
    )
    .await?;

    // Ensure the registry is populated — `from_config_for_agent`
    // errors for any non-orchestrator id when the global registry
    // hasn't been initialised.
    AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .context("initialising AgentDefinitionRegistry for prompt dump")?;

    if options.agent_id == INTEGRATIONS_AGENT_ID {
        let toolkit = options.toolkit.as_deref().ok_or_else(|| {
            anyhow!(
                "integrations_agent requires a `toolkit` argument — e.g. \
                 `gmail`, `notion`. See `composio list_connection` for \
                 the currently-connected toolkits."
            )
        })?;
        render_integrations_agent(&config, toolkit).await
    } else {
        render_via_session(&config, &options.agent_id).await
    }
}

/// Dump every registered agent's system prompt in one shot.
///
/// The synthetic `fork` archetype is skipped (byte-stable replay, no
/// standalone prompt). `integrations_agent` is expanded into one dump
/// per currently-connected Composio toolkit — if the user has gmail +
/// notion connected, `dump_all_agent_prompts` returns an entry for
/// `integrations_agent@gmail` and another for `integrations_agent@notion`.
/// When no toolkit is connected, `integrations_agent` is omitted
/// entirely (there's nothing meaningful to render).
///
/// Order follows [`AgentDefinitionRegistry::list`], with
/// `integrations_agent` replaced in place by its per-toolkit expansion.
pub async fn dump_all_agent_prompts(
    workspace_dir_override: Option<PathBuf>,
    model_override: Option<String>,
) -> Result<Vec<DumpedPrompt>> {
    let config = load_dump_config(workspace_dir_override, model_override).await?;

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
        if id == INTEGRATIONS_AGENT_ID {
            let toolkits = connected_toolkits_for(&config).await?;
            if toolkits.is_empty() {
                log::info!(
                    "[debug_dump] skipping integrations_agent — no connected toolkits"
                );
                continue;
            }
            for toolkit in toolkits {
                let dumped = render_integrations_agent(&config, &toolkit)
                    .await
                    .with_context(|| {
                        format!("rendering integrations_agent prompt for toolkit `{toolkit}`")
                    })?;
                results.push(dumped);
            }
            continue;
        }

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
    model_override: Option<String>,
) -> Result<Config> {
    let mut config = Config::load_or_init()
        .await
        .context("loading Config for prompt dump")?;
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
    let mut agent = Agent::from_config_for_agent(config, agent_id)
        .with_context(|| format!("building session agent for `{agent_id}`"))?;

    // Match turn-1 behaviour: fetch the user's active Composio
    // connections so the rendered prompt mirrors what the LLM actually
    // sees. Best-effort — failures degrade to an empty integration
    // list, same as the live runtime.
    agent.fetch_connected_integrations().await;

    let text = agent
        .build_system_prompt(LearnedContextData::default())
        .with_context(|| format!("rendering system prompt for `{agent_id}`"))?;

    let tools = agent.tools();
    let tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
    let skill_tool_count = tools
        .iter()
        .filter(|t| t.category() == ToolCategory::Skill)
        .count();

    Ok(DumpedPrompt {
        agent_id: agent_id.to_string(),
        toolkit: None,
        mode: "session",
        model: agent.model_name().to_string(),
        workspace_dir: agent.workspace_dir().to_path_buf(),
        text,
        tool_names,
        skill_tool_count,
    })
}

/// Render the integrations_agent prompt bound to a single Composio
/// toolkit. Mirrors the subagent_runner's per-toolkit path: strips
/// Skill-category parent tools, injects one [`ComposioActionTool`] per
/// action in the toolkit, and narrows the `connected_integrations`
/// slice to only the requested toolkit before calling the agent's
/// dynamic prompt builder.
async fn render_integrations_agent(config: &Config, toolkit: &str) -> Result<DumpedPrompt> {
    let mut agent = Agent::from_config_for_agent(config, INTEGRATIONS_AGENT_ID)
        .with_context(|| format!("building integrations_agent session for `{toolkit}`"))?;
    agent.fetch_connected_integrations().await;

    let mut integration = agent
        .connected_integrations()
        .iter()
        .find(|ci| ci.connected && ci.toolkit.eq_ignore_ascii_case(toolkit))
        .cloned()
        .ok_or_else(|| {
            let connected: Vec<String> = agent
                .connected_integrations()
                .iter()
                .filter(|ci| ci.connected)
                .map(|ci| ci.toolkit.clone())
                .collect();
            anyhow!(
                "toolkit `{toolkit}` is not connected. Connected toolkits: [{}]",
                connected.join(", ")
            )
        })?;

    let composio_client = agent
        .composio_client()
        .cloned()
        .ok_or_else(|| anyhow!("composio client unavailable — is the user signed in?"))?;

    // Refresh the action catalogue for this toolkit at prompt-generation
    // time so the dump reflects the **current** backend state rather
    // than the session-start bulk fetch's snapshot (which can return an
    // empty list for some toolkits even when the per-toolkit endpoint
    // returns actions).
    integration.tools = crate::openhuman::composio::fetch_toolkit_actions(
        &composio_client,
        &integration.toolkit,
    )
    .await
    .with_context(|| {
        format!(
            "fetching fresh action catalogue for toolkit `{}`",
            integration.toolkit
        )
    })?;

    // Build the tool list that subagent_runner would produce for a
    // real spawn. Tool visibility honours the TOML scope on the
    // `integrations_agent` definition — `named = [...]` narrows, and
    // `wildcard = {}` means "every parent tool". The dynamic
    // ComposioActionTools for the bound toolkit are added after.
    let definition_snapshot = AgentDefinitionRegistry::global()
        .and_then(|reg| reg.get(INTEGRATIONS_AGENT_ID).cloned())
        .ok_or_else(|| anyhow!("integrations_agent definition missing from registry"))?;
    let base_tools: Vec<Box<dyn Tool>> = match &definition_snapshot.tools {
        crate::openhuman::agent::harness::definition::ToolScope::Named(names) => {
            let allow: HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
            agent
                .tools()
                .iter()
                .filter(|t| allow.contains(t.name()))
                .map(|t| clone_tool_as_prompt_proxy(t.as_ref()))
                .collect()
        }
        crate::openhuman::agent::harness::definition::ToolScope::Wildcard => agent
            .tools()
            .iter()
            .map(|t| clone_tool_as_prompt_proxy(t.as_ref()))
            .collect(),
    };
    let action_tools: Vec<Box<dyn Tool>> = integration
        .tools
        .iter()
        .map(|action| -> Box<dyn Tool> {
            Box::new(ComposioActionTool::new(
                composio_client.clone(),
                action.name.clone(),
                action.description.clone(),
                action.parameters.clone(),
            ))
        })
        .collect();
    let mut rendered_tools: Vec<Box<dyn Tool>> = base_tools;
    rendered_tools.extend(action_tools);

    let prompt_tools: Vec<PromptTool<'_>> = rendered_tools
        .iter()
        .map(|t| PromptTool {
            name: t.name(),
            description: t.description(),
            parameters_schema: Some(t.parameters_schema().to_string()),
        })
        .collect();

    // Narrow the connected_integrations slice to just the bound
    // toolkit so the prompt's Connected Integrations / tool catalogue
    // doesn't leak peer toolkits into this sub-agent's context.
    let narrow_integrations = vec![integration.clone()];

    let registry = AgentDefinitionRegistry::global()
        .ok_or_else(|| anyhow!("AgentDefinitionRegistry missing after init"))?;
    let definition: AgentDefinition = registry
        .get(INTEGRATIONS_AGENT_ID)
        .cloned()
        .ok_or_else(|| anyhow!("integrations_agent definition not in registry"))?;
    let build = match &definition.system_prompt {
        PromptSource::Dynamic(f) => *f,
        _ => {
            return Err(anyhow!(
                "integrations_agent must use PromptSource::Dynamic; got {:?}",
                match &definition.system_prompt {
                    PromptSource::Inline(_) => "Inline",
                    PromptSource::File { .. } => "File",
                    PromptSource::Dynamic(_) => "Dynamic",
                }
            ));
        }
    };

    let empty_visible: HashSet<String> = HashSet::new();
    let model_name = definition.model.resolve(agent.model_name()).to_string();
    let ctx = PromptContext {
        workspace_dir: agent.workspace_dir(),
        model_name: &model_name,
        agent_id: INTEGRATIONS_AGENT_ID,
        tools: &prompt_tools,
        skills: agent.skills(),
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &empty_visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &narrow_integrations,
        include_profile: !definition.omit_profile,
        include_memory_md: !definition.omit_memory_md,
    };

    let text = build(&ctx)
        .with_context(|| format!("building integrations_agent prompt for toolkit `{toolkit}`"))?;

    let tool_names: Vec<String> = rendered_tools.iter().map(|t| t.name().to_string()).collect();
    let skill_tool_count = rendered_tools
        .iter()
        .filter(|t| t.category() == ToolCategory::Skill)
        .count();

    Ok(DumpedPrompt {
        agent_id: INTEGRATIONS_AGENT_ID.to_string(),
        toolkit: Some(integration.toolkit.clone()),
        mode: "session",
        model: model_name,
        workspace_dir: agent.workspace_dir().to_path_buf(),
        text,
        tool_names,
        skill_tool_count,
    })
}

/// Wrap a `&dyn Tool` as a `Box<dyn Tool>` proxy that forwards
/// `name()` / `description()` / `parameters_schema()` / `category()`
/// — enough surface for prompt rendering. `execute` is intentionally
/// left as a no-op error since dumps never call it.
fn clone_tool_as_prompt_proxy(source: &dyn Tool) -> Box<dyn Tool> {
    Box::new(PromptProxyTool {
        name: source.name().to_string(),
        description: source.description().to_string(),
        schema: source.parameters_schema(),
        category: source.category(),
    })
}

struct PromptProxyTool {
    name: String,
    description: String,
    schema: serde_json::Value,
    category: ToolCategory,
}

#[async_trait::async_trait]
impl Tool for PromptProxyTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }
    fn category(&self) -> ToolCategory {
        self.category
    }
    fn permission_level(&self) -> crate::openhuman::tools::PermissionLevel {
        crate::openhuman::tools::PermissionLevel::None
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> anyhow::Result<crate::openhuman::tools::ToolResult> {
        Err(anyhow!(
            "PromptProxyTool (`{}`) is a render-only stub — execute is not callable",
            self.name
        ))
    }
}

/// Return the slugs of every currently-connected Composio toolkit.
/// Used by [`dump_all_agent_prompts`] to decide how many times to
/// render `integrations_agent`. Empty when the user is not signed in
/// or has no active connections.
async fn connected_toolkits_for(config: &Config) -> Result<Vec<String>> {
    // Spin up a throwaway integrations_agent session just so we can
    // reuse its `fetch_connected_integrations` cache — the call is
    // deduped backend-side via `INTEGRATIONS_CACHE`, so repeated
    // invocations in `dump_all_agent_prompts` only hit the wire once.
    let mut agent = Agent::from_config_for_agent(config, INTEGRATIONS_AGENT_ID)
        .with_context(|| "building integrations_agent probe session for toolkit discovery")?;
    agent.fetch_connected_integrations().await;
    Ok(agent
        .connected_integrations()
        .iter()
        .filter(|ci| ci.connected)
        .map(|ci| ci.toolkit.clone())
        .collect())
}

// `config` usage is currently limited to plumbing; keep the path-level
// reference alive so future hooks (overrides, scoped workspaces) can
// extend from here without touching every call site.
#[allow(dead_code)]
fn _keep_path_imports_alive(_p: &Path) {}
