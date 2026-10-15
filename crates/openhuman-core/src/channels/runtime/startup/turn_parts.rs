//! What a channel turn is built from: the chat model and provider, the tool
//! registry, the system prompt, and the `ChannelRuntimeContext` they make up.
//!
//! `start_channels` builds these once for the process. The hosted-channel
//! relay (`channels::providers::relay`) builds them per inbound message from
//! the caller's own config and policy, so a relayed turn never runs with the
//! operator's tools or workspace.

use super::chat_workload::{resolve_chat_workload, ChatWorkloadResolution};
use super::prompt::format_access_context;
use crate::agent::host_runtime;
use crate::channels::context::ChannelRuntimeContext;
use crate::channels::system_prompt::{ChannelPromptInputs, ChannelSystemPrompt};
use crate::config::Config;
use crate::inference::provider;
use crate::security::SecurityPolicy;
use crate::tools;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tinyskills::Skill as Workflow;
use tinytools::Tool;

/// The process-independent pieces of a channel turn.
pub(crate) struct ChannelTurnParts {
    pub(crate) model: String,
    pub(crate) provider_name: String,
    pub(crate) provider_runtime_options: provider::ProviderRuntimeOptions,
    pub(crate) tools_registry: Arc<Vec<Box<dyn Tool>>>,
    pub(crate) system_prompt: ChannelSystemPrompt,
    pub(crate) skills: Vec<Workflow>,
}

/// Which tool descriptions the channel prompt lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptToolDescs {
    /// The fixed list `start_channels` has always rendered.
    Fixed,
    /// The fixed list, narrowed to tools actually in the registry. A caller
    /// whose registry is filtered (a SaaS user's) is not told about tools it
    /// does not have.
    Registered,
}

/// The fixed tool descriptions of the channel prompt.
fn default_tool_descs(config: &Config) -> Vec<(&'static str, &'static str)> {
    let mut tool_descs: Vec<(&str, &str)> = vec![
        (
            "shell",
            "Execute terminal commands. Use when: running local checks, build/test commands, diagnostics. Don't use when: a safer dedicated tool exists, or command is destructive without approval.",
        ),
        (
            "file_read",
            "Read file contents. Use when: inspecting project files, configs, logs. Don't use when: a targeted search is enough.",
        ),
        (
            "file_write",
            "Write file contents. Use when: applying focused edits, scaffolding files, updating docs/code. Don't use when: side effects are unclear or file ownership is uncertain.",
        ),
        (
            "memory",
            "Long-term memory: recall, fetch, learn, forget. Use when: retrieving prior decisions, preferences or history beyond the <memory-context> each turn already carries, or preserving a durable fact. Don't use when: the answer is already in context or the information is transient.",
        ),
    ];

    if config.browser.enabled {
        tool_descs.push((
            "browser_open",
            "Open approved HTTPS URLs in Brave Browser (allowlist-only, no scraping)",
        ));
    }
    // Composio tool descriptions are intentionally excluded from the main
    // agent prompt — integration actions are reached through tool search.
    tool_descs.push((
        "schedule",
        "Manage scheduled tasks (create/list/get/cancel/pause/resume). Supports recurring cron and one-shot delays.",
    ));
    tool_descs.push((
        "pushover",
        "Send a Pushover notification to your device. Requires PUSHOVER_TOKEN and PUSHOVER_USER_KEY in .env file.",
    ));
    if !config.agents.is_empty() {
        tool_descs.push((
            "delegate",
            "Delegate a subtask to a specialized agent. Use when: a task benefits from a different model (e.g. fast summarization, deep reasoning, code generation). The sub-agent runs a single prompt and returns its response.",
        ));
    }
    tool_descs
}

/// Build the model, tools and prompt of a channel turn from `config`, under
/// `security`. Side-effect free apart from the workspace's audit logger: it
/// installs no process-global policy and registers no subscriber.
pub(crate) fn build_channel_turn_parts(
    config: &Config,
    security: &Arc<SecurityPolicy>,
    descs: PromptToolDescs,
) -> Result<ChannelTurnParts> {
    let provider_runtime_options = provider::ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        reasoning_enabled: config.runtime.reasoning_enabled,
    };
    let (model, provider_name) = match resolve_chat_workload(config) {
        ChatWorkloadResolution::Cloud => {
            let (_chat, model) = provider::create_chat_model_with_model_id(
                "chat",
                config,
                config.default_temperature,
            )?;
            (model, provider::INFERENCE_BACKEND_ID.to_string())
        }
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            tracing::info!(
                chat_provider = %provider_string,
                slug = %slug,
                "[channels][startup] chat workload routed to per-workload provider — building dedicated channel provider"
            );
            let (_chat, model_id) = provider::create_chat_model_with_model_id(
                "chat",
                config,
                config.default_temperature,
            )?;
            (model_id, slug)
        }
    };

    let runtime: Arc<dyn host_runtime::RuntimeAdapter> = Arc::from(host_runtime::create_runtime(
        &config.runtime,
        config.shell.hide_window,
    )?);
    // Phase 1 of #1401: audit logger is wired with defaults so emission paths
    // are exercised at runtime. The logger is workspace-scoped and shared, so
    // concurrent sessions append to one `audit.log` without racing on rotation.
    let audit = crate::security::get_or_create_workspace_audit_logger(
        config.security.audit.clone(),
        config.workspace_dir.clone(),
    )?;
    let workspace = config.workspace_dir.clone();
    let capabilities = crate::profiles::capabilities::ProfileCapabilities::current();
    let tools_registry = Arc::new(if capabilities.is_some() {
        // The profile host route builds the selected belt once with the exact
        // durable thread. No second, unnamed factory invocation is needed here.
        Vec::new()
    } else {
        tools::ops::all_tools_with_runtime(
            Arc::new(config.clone()),
            security,
            runtime,
            audit,
            // `all_tools_with_runtime` no longer takes a memory handle — the two
            // tools that needed one resolve the guarded driver per call.
            &config.browser,
            &config.http_request,
            &config.action_dir,
            &config.agents,
            config,
            None,
        )
    });

    let skills = if capabilities.is_some() {
        Vec::new()
    } else {
        crate::skills::load_workflow_metadata(&workspace)
    };

    let tool_descs: Vec<(String, String)> = if capabilities.is_some() {
        tools_registry
            .iter()
            .map(|tool| (tool.name().to_owned(), tool.description().to_owned()))
            .collect()
    } else {
        default_tool_descs(config)
            .into_iter()
            .filter(|(name, _)| {
                descs == PromptToolDescs::Fixed || tools_registry.iter().any(|t| t.name() == *name)
            })
            .map(|(name, desc)| (name.to_string(), desc.to_string()))
            .collect()
    };

    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    // Filter out Workflow-category tools (e.g. Composio) from the
    // main agent prompt — integration actions are reached through tool search.
    let non_skill_specs: Vec<tinytools::ToolSpec> = tools_registry
        .iter()
        .filter(|t| t.category() != tinytools::ToolCategory::Workflow)
        .map(|tool| tool.spec())
        .collect();
    // Everything after the rendered prompt is fixed for the runtime: the
    // tool-instruction block, then the model's current filesystem access
    // boundaries so it self-limits (advisory only — the SecurityPolicy
    // enforces these regardless).
    let mut prompt_suffix = tinytools_agent::dialect::XmlDialect::instructions(&non_skill_specs);
    prompt_suffix.push_str(&format_access_context(security));
    // The prompt itself is rendered here for the current identity and
    // re-rendered whenever the active profile or an identity file changes
    // (#6027, #6028). `channel_name = None`: the runtime wires up multiple
    // providers in parallel, so the capability block keeps its
    // platform-agnostic "messaging bot" phrasing.
    let system_prompt = ChannelSystemPrompt::refreshing(ChannelPromptInputs {
        workspace_dir: workspace,
        model: model.clone(),
        tool_descs,
        skills: skills.clone(),
        bootstrap_max_chars,
        suffix: prompt_suffix,
    });

    Ok(ChannelTurnParts {
        model,
        provider_name,
        provider_runtime_options,
        tools_registry,
        system_prompt,
        skills,
    })
}

/// The runtime context for `parts`, delivering replies to `channels_by_name`
/// and starting with no per-chat history.
pub(crate) fn runtime_context(
    config: &Config,
    parts: ChannelTurnParts,
    channels_by_name: Arc<HashMap<String, Arc<dyn crate::channels::Channel>>>,
    message_timeout_secs: u64,
) -> ChannelRuntimeContext {
    ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name,
        turn_model_source: None,
        default_provider: Arc::new(parts.provider_name),
        tools_registry: parts.tools_registry,
        system_prompt: parts.system_prompt,
        model: Arc::new(parts.model),
        temperature: config.default_temperature,
        max_tool_iterations: config.agent.max_tool_iterations,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: config.api_url.clone(),
        inference_url: config.inference_url.clone(),
        reliability: Arc::new(config.reliability.clone()),
        provider_runtime_options: parts.provider_runtime_options,
        workspace_dir: Arc::new(config.workspace_dir.clone()),
        message_timeout_secs,
        multimodal: config.multimodal.clone(),
        multimodal_files: config.multimodal_files.clone(),
        // Crate-native turn models for the channel turn (Phase 3 P3-B).
        config: Some(Arc::new(config.clone())),
    }
}
