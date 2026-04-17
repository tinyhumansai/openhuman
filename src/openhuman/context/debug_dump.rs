//! Debug helper that renders the exact system prompt the context engine
//! would produce for a given agent.
//!
//! Entry points:
//! * [`dump_agent_prompt`] — dump a single agent by id.
//! * [`dump_all_agent_prompts`] — dump every registered agent in one call.
//!
//! Both share [`build_dump_env`] which assembles the tool registry,
//! connected-integration list, and agent-definition registry. Each
//! agent then flows through [`render_subagent_dump`], which mirrors
//! `subagent_runner::run_typed_mode` step-for-step — no parallel
//! rendering path, no case-by-case per-agent branching.
//!
//! There is intentionally **no** "main" / orchestrator-specific dump
//! path: the orchestrator is just another registered agent and renders
//! through the same pipeline. Scripts looking for "main" should pass
//! `--agent orchestrator` instead.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, PromptSource,
};
use crate::openhuman::agent::harness::subagent_runner::{filter_tool_indices, load_prompt_source};
use crate::openhuman::agent::host_runtime::{self, RuntimeAdapter};
use crate::openhuman::composio::client::ComposioClient;
use crate::openhuman::composio::tools::{
    ComposioAuthorizeTool, ComposioExecuteTool, ComposioListConnectionsTool,
    ComposioListToolkitsTool, ComposioListToolsTool,
};
use crate::openhuman::config::Config;
use crate::openhuman::context::prompt::{
    extract_cache_boundary, render_subagent_system_prompt, ConnectedIntegration,
    LearnedContextData, PromptContext, PromptTool, SubagentRenderOptions, ToolCallFormat,
};
use crate::openhuman::integrations::IntegrationClient;
use crate::openhuman::memory::{self, Memory};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::{self, Tool, ToolCategory};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Inputs for [`dump_agent_prompt`].
#[derive(Debug, Clone)]
pub struct DumpPromptOptions {
    /// Target agent id (any id registered in [`AgentDefinitionRegistry`]).
    pub agent_id: String,
    /// Optional per-spawn skill filter override (e.g. `Some("notion".into())`).
    pub skill_filter: Option<String>,
    /// Optional override for the workspace directory.
    pub workspace_dir_override: Option<PathBuf>,
    /// Optional override for the resolved model name.
    pub model_override: Option<String>,
    /// When `true`, inject the five Composio meta-tool stubs into the
    /// registry even if the user isn't signed in. Render-time only —
    /// the stubs are safe to inspect but must not be `.execute()`d.
    pub stub_composio: bool,
}

impl DumpPromptOptions {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            skill_filter: None,
            workspace_dir_override: None,
            model_override: None,
            stub_composio: false,
        }
    }
}

/// Result of a single prompt dump.
#[derive(Debug, Clone)]
pub struct DumpedPrompt {
    /// Echoed from [`DumpPromptOptions::agent_id`].
    pub agent_id: String,
    /// Always `"subagent"` — there is no main-agent dump path anymore.
    pub mode: &'static str,
    /// Resolved model name.
    pub model: String,
    /// Workspace directory used for identity file injection.
    pub workspace_dir: PathBuf,
    /// The final rendered system prompt (cache-boundary marker stripped;
    /// the offset is in `cache_boundary`).
    pub text: String,
    /// Byte offset of the cache boundary, if the builder emitted one.
    pub cache_boundary: Option<usize>,
    /// Tool names that made it into the rendered prompt, in order.
    pub tool_names: Vec<String>,
    /// Number of `ToolCategory::Skill` tools in the dump.
    pub skill_tool_count: usize,
}

/// Render and return the system prompt for a single agent.
pub async fn dump_agent_prompt(options: DumpPromptOptions) -> Result<DumpedPrompt> {
    let env = build_dump_env(
        options.workspace_dir_override.clone(),
        options.model_override.clone(),
        options.stub_composio,
    )
    .await?;

    let definition = env.registry.get(&options.agent_id).cloned().ok_or_else(|| {
        let known: Vec<&str> = env.registry.list().iter().map(|d| d.id.as_str()).collect();
        anyhow!(
            "unknown agent id `{}`. Known agents: [{}]",
            options.agent_id,
            known.join(", ")
        )
    })?;

    render_subagent_dump(
        &definition,
        &env.workspace_dir,
        &env.model_name,
        &env.tools_vec,
        options.skill_filter.as_deref(),
        &env.connected_integrations,
    )
}

/// Dump every registered agent's system prompt in one shot.
///
/// The synthetic `fork` archetype is skipped (byte-stable replay, no
/// standalone prompt). Order follows [`AgentDefinitionRegistry::list`].
pub async fn dump_all_agent_prompts(
    workspace_dir_override: Option<PathBuf>,
    model_override: Option<String>,
    stub_composio: bool,
) -> Result<Vec<DumpedPrompt>> {
    let env = build_dump_env(workspace_dir_override, model_override, stub_composio).await?;

    let mut results = Vec::with_capacity(env.registry.len());
    for def in env.registry.list() {
        if def.id == "fork" {
            continue;
        }
        let dumped = render_subagent_dump(
            def,
            &env.workspace_dir,
            &env.model_name,
            &env.tools_vec,
            None,
            &env.connected_integrations,
        )
        .with_context(|| format!("rendering prompt for agent `{}`", def.id))?;
        results.push(dumped);
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Shared environment setup
// ---------------------------------------------------------------------------

struct DumpEnv {
    workspace_dir: PathBuf,
    model_name: String,
    tools_vec: Vec<Box<dyn Tool>>,
    connected_integrations: Vec<ConnectedIntegration>,
    registry: AgentDefinitionRegistry,
}

async fn build_dump_env(
    workspace_dir_override: Option<PathBuf>,
    model_override: Option<String>,
    stub_composio: bool,
) -> Result<DumpEnv> {
    let mut config = Config::load_or_init()
        .await
        .context("loading Config for prompt dump")?;
    config.apply_env_overrides();
    if let Some(override_dir) = workspace_dir_override {
        config.workspace_dir = override_dir;
    }
    let workspace_dir = config.workspace_dir.clone();
    std::fs::create_dir_all(&workspace_dir).ok();

    let model_name = model_override.unwrap_or_else(|| {
        config
            .default_model
            .clone()
            .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string())
    });

    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &workspace_dir,
    ));
    let runtime: Arc<dyn RuntimeAdapter> = Arc::from(
        host_runtime::create_runtime(&config.runtime)
            .context("creating host runtime for prompt dump")?,
    );
    let mem: Arc<dyn Memory> = Arc::from(
        memory::create_memory(&config.memory, &workspace_dir, config.api_key.as_deref())
            .context("creating memory backend for prompt dump")?,
    );

    let composio_key = config.composio.enabled.then_some(config.composio.api_key.as_deref()).flatten();
    let composio_entity_id = config
        .composio
        .enabled
        .then_some(config.composio.entity_id.as_str());

    let mut tools_vec: Vec<Box<dyn Tool>> = tools::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        mem,
        composio_key,
        composio_entity_id,
        &config.browser,
        &config.http_request,
        &workspace_dir,
        &config.agents,
        config.api_key.as_deref(),
        &config,
    );
    if stub_composio && !tools_vec.iter().any(|t| t.name().starts_with("composio_")) {
        tools_vec.extend(build_composio_stub_tools());
    }

    let connected_integrations =
        crate::openhuman::composio::fetch_connected_integrations(&config).await;
    let registry = AgentDefinitionRegistry::load(&workspace_dir)
        .context("loading agent definition registry for prompt dump")?;

    Ok(DumpEnv {
        workspace_dir,
        model_name,
        tools_vec,
        connected_integrations,
        registry,
    })
}

/// Build the five Composio meta-tools wired to a dummy backend URL. Safe
/// for rendering (`name()` / `description()` / `parameters_schema()`
/// are pure); **never** call `.execute()` on these stubs.
fn build_composio_stub_tools() -> Vec<Box<dyn Tool>> {
    let inner = Arc::new(IntegrationClient::new(
        "http://127.0.0.1:0".to_string(),
        "debug-dump-stub-token".to_string(),
    ));
    let client = ComposioClient::new(inner);
    vec![
        Box::new(ComposioListToolkitsTool::new(client.clone())),
        Box::new(ComposioListConnectionsTool::new(client.clone())),
        Box::new(ComposioAuthorizeTool::new(client.clone())),
        Box::new(ComposioListToolsTool::new(client.clone())),
        Box::new(ComposioExecuteTool::new(client)),
    ]
}

// ---------------------------------------------------------------------------
// Per-agent rendering
// ---------------------------------------------------------------------------

/// Render a single agent's prompt. Mirrors `run_typed_mode` exactly:
/// resolve model → filter tools → build live `PromptContext` → dispatch
/// on the prompt source (Dynamic → `build()`; Inline/File → legacy
/// section wrap).
fn render_subagent_dump(
    definition: &AgentDefinition,
    workspace_dir: &Path,
    model_name: &str,
    tools_vec: &[Box<dyn Tool>],
    skill_filter_override: Option<&str>,
    connected_integrations: &[ConnectedIntegration],
) -> Result<DumpedPrompt> {
    let model = definition.model.resolve(model_name);
    let effective_skill_filter = skill_filter_override.or(definition.skill_filter.as_deref());
    let allowed_indices = filter_tool_indices(
        tools_vec,
        &definition.tools,
        &definition.disallowed_tools,
        effective_skill_filter,
        definition.category_filter,
    );

    let prompt_tools: Vec<PromptTool<'_>> = allowed_indices
        .iter()
        .map(|&i| PromptTool {
            name: tools_vec[i].name(),
            description: tools_vec[i].description(),
            parameters_schema: Some(tools_vec[i].parameters_schema().to_string()),
        })
        .collect();
    let empty_visible: std::collections::HashSet<String> = std::collections::HashSet::new();
    let prompt_ctx = PromptContext {
        workspace_dir,
        model_name: &model,
        agent_id: &definition.id,
        tools: &prompt_tools,
        skills: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &empty_visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations,
        include_profile: !definition.omit_profile,
        include_memory_md: !definition.omit_memory_md,
    };

    let rendered = match &definition.system_prompt {
        PromptSource::Dynamic(build) => {
            let text = build(&prompt_ctx)
                .with_context(|| format!("building dynamic prompt for {}", definition.id))?;
            extract_cache_boundary(&text)
        }
        _ => {
            let archetype_body = load_prompt_source(&definition.system_prompt, &prompt_ctx)
                .with_context(|| format!("loading prompt for {}", definition.id))?;
            let options = SubagentRenderOptions::from_definition_flags(
                definition.omit_identity,
                definition.omit_safety_preamble,
                definition.omit_skills_catalog,
                definition.omit_profile,
                definition.omit_memory_md,
            );
            let raw = render_subagent_system_prompt(
                workspace_dir,
                &model,
                &allowed_indices,
                tools_vec,
                &[],
                &archetype_body,
                options,
                ToolCallFormat::PFormat,
                connected_integrations,
            );
            extract_cache_boundary(&raw)
        }
    };

    let tool_names: Vec<String> = allowed_indices
        .iter()
        .map(|&i| tools_vec[i].name().to_string())
        .collect();
    let skill_tool_count = allowed_indices
        .iter()
        .filter(|&&i| tools_vec[i].category() == ToolCategory::Skill)
        .count();

    Ok(DumpedPrompt {
        agent_id: definition.id.clone(),
        mode: "subagent",
        model,
        workspace_dir: workspace_dir.to_path_buf(),
        text: rendered.text,
        cache_boundary: rendered.cache_boundary,
        tool_names,
        skill_tool_count,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::{
        DefinitionSource, ModelSpec, SandboxMode, ToolScope,
    };
    use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
    use async_trait::async_trait;

    struct StubTool {
        name: &'static str,
        category: ToolCategory,
    }

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "stub tool used by debug_dump tests"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        fn category(&self) -> ToolCategory {
            self.category
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::None
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
    }

    fn stub_agent(id: &str, scope: ToolScope, category: Option<ToolCategory>) -> AgentDefinition {
        AgentDefinition {
            id: id.into(),
            when_to_use: "t".into(),
            display_name: None,
            system_prompt: PromptSource::Inline(format!("# {id}\n\nBody.")),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            omit_skills_catalog: true,
            omit_profile: true,
            omit_memory_md: true,
            model: ModelSpec::Inherit,
            temperature: 0.0,
            tools: scope,
            disallowed_tools: vec![],
            skill_filter: None,
            category_filter: category,
            extra_tools: vec![],
            max_iterations: 2,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
            uses_fork_context: false,
            subagents: vec![],
            delegate_name: None,
            source: DefinitionSource::Builtin,
        }
    }

    #[test]
    fn dump_prompt_options_new_sets_expected_defaults() {
        let opts = DumpPromptOptions::new("planner");
        assert_eq!(opts.agent_id, "planner");
        assert!(opts.skill_filter.is_none());
        assert!(!opts.stub_composio);
    }

    #[test]
    fn composio_stub_tools_have_expected_names() {
        let stubs = build_composio_stub_tools();
        let names: Vec<&str> = stubs.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"composio_list_toolkits"));
        assert!(names.contains(&"composio_list_connections"));
        assert!(names.contains(&"composio_authorize"));
        assert!(names.contains(&"composio_list_tools"));
        assert!(names.contains(&"composio_execute"));
    }

    #[test]
    fn filter_respects_named_scope_and_disallowed_tools() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool {
                name: "shell",
                category: ToolCategory::System,
            }),
            Box::new(StubTool {
                name: "memory_recall",
                category: ToolCategory::System,
            }),
            Box::new(StubTool {
                name: "notion__create_page",
                category: ToolCategory::Skill,
            }),
        ];
        let indices = filter_tool_indices(
            &tools,
            &ToolScope::Named(vec!["memory_recall".into(), "notion__create_page".into()]),
            &["notion__create_page".into()],
            None,
            None,
        );
        let names: Vec<&str> = indices.iter().map(|&i| tools[i].name()).collect();
        assert_eq!(names, vec!["memory_recall"]);
    }

    #[test]
    fn render_subagent_dump_inline_source_produces_nonempty_prompt() {
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_debug_inline_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(StubTool {
            name: "shell",
            category: ToolCategory::System,
        })];
        let def = stub_agent("inline_agent", ToolScope::Wildcard, None);
        let dumped =
            render_subagent_dump(&def, &workspace, "reasoning-v1", &tools, None, &[]).unwrap();
        assert_eq!(dumped.mode, "subagent");
        assert!(dumped.text.contains("inline_agent"));
        assert!(dumped.text.contains("## Tools"));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn render_subagent_dump_skill_category_narrows_tools() {
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_debug_skill_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool {
                name: "shell",
                category: ToolCategory::System,
            }),
            Box::new(StubTool {
                name: "notion__create_page",
                category: ToolCategory::Skill,
            }),
        ];
        let def = stub_agent(
            "skills_only",
            ToolScope::Wildcard,
            Some(ToolCategory::Skill),
        );
        let dumped =
            render_subagent_dump(&def, &workspace, "reasoning-v1", &tools, None, &[]).unwrap();
        assert_eq!(dumped.tool_names, vec!["notion__create_page"]);
        assert_eq!(dumped.skill_tool_count, 1);
        let _ = std::fs::remove_dir_all(workspace);
    }
}
