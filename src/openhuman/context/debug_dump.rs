//! Debug helper that renders the exact system prompt the context engine
//! would produce for a given agent.
//!
//! This is the canonical entry point shared by:
//!
//! * the `openhuman agent dump-prompt` CLI (see [`crate::core::agent_cli`])
//! * `scripts/debug-agent-prompts.sh` (loops over every built-in)
//! * any future JSON-RPC / tests that need to inspect the assembled prompt
//!
//! The function assembles a **real** tool registry (via
//! [`crate::openhuman::tools::all_tools_with_runtime`]) and a **real**
//! [`AgentDefinitionRegistry`], then feeds them through the exact same
//! prompt builders used at runtime — so what you see here is byte-identical
//! to what the LLM would see at spawn time.
//!
//! # Targets
//!
//! * `"main"` (or any non-matching id when `--force-main` is set) →
//!   the orchestrator / main-agent prompt assembled via
//!   [`super::prompt::SystemPromptBuilder::with_defaults`]. This includes
//!   the workspace identity files, tools visible to the main agent, and
//!   the skills catalogue.
//!
//! * Any built-in or custom sub-agent id (e.g. `"skills_agent"`,
//!   `"orchestrator"`, `"code_executor"`) →
//!   [`super::prompt::render_subagent_system_prompt`] with the narrow
//!   tool filter and per-definition `omit_*` flags the real runner applies.
//!
//! # Composio coverage
//!
//! When `Config::composio.enabled` is true, the Composio meta-tools
//! (`composio_list_toolkits`, `composio_list_connections`,
//! `composio_authorize`, `composio_list_tools`, `composio_execute`) are
//! registered into the tool list with `ToolCategory::Skill`. Agents whose
//! definition sets `category_filter = Skill` (notably `skills_agent`) will
//! render those tools in their system prompt, so this dump is the fastest
//! way to verify Composio is reaching the skills agent end-to-end.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, PromptSource, ToolScope,
};
use crate::openhuman::agent::host_runtime::{self, RuntimeAdapter};
use crate::openhuman::config::Config;
use crate::openhuman::context::prompt::{
    extract_cache_boundary, render_subagent_system_prompt, LearnedContextData, PromptContext,
    PromptTool, SubagentRenderOptions, SystemPromptBuilder,
    USER_MEMORY_PER_NAMESPACE_MAX_CHARS, USER_MEMORY_TOTAL_MAX_CHARS,
};
use crate::openhuman::composio::client::ComposioClient;
use crate::openhuman::composio::tools::{
    ComposioAuthorizeTool, ComposioExecuteTool, ComposioListConnectionsTool,
    ComposioListToolkitsTool, ComposioListToolsTool,
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
    /// Target agent id — either `"main"` for the main/orchestrator agent,
    /// or any id registered in [`AgentDefinitionRegistry`].
    pub agent_id: String,
    /// Optional per-spawn skill filter override (e.g. `Some("notion".into())`).
    /// Ignored for `"main"`.
    pub skill_filter: Option<String>,
    /// Optional override for the workspace directory. When `None`, the
    /// value from the loaded [`Config`] is used.
    pub workspace_dir_override: Option<PathBuf>,
    /// Optional override for the resolved model name. When `None`, the
    /// value from the loaded [`Config`] is used. Only affects the
    /// `## Runtime` line of the rendered prompt.
    pub model_override: Option<String>,
    /// When `true`, always inject the five Composio meta-tool stubs
    /// (`composio_list_toolkits`, `composio_list_connections`,
    /// `composio_authorize`, `composio_list_tools`, `composio_execute`)
    /// into the tool registry before rendering, even if the user is not
    /// signed in or `config.composio` is disabled.
    ///
    /// This is strictly a **dump-time** debug aid: the stubs are real
    /// `Tool` impls so their names, descriptions, and parameter schemas
    /// render byte-identically to what a signed-in user would see — but
    /// calling `execute()` on them would hit a dummy localhost endpoint,
    /// so they're safe for prompt inspection only, not for running
    /// agents against. Use this to answer "what would `skills_agent`
    /// see if Composio were reachable right now?" on a fresh dev
    /// machine without wiring up OAuth.
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
    /// `"main"` or `"subagent"` — which rendering path produced `text`.
    pub mode: &'static str,
    /// Resolved model name used in the `## Runtime` section.
    pub model: String,
    /// Workspace directory used for identity file injection.
    pub workspace_dir: PathBuf,
    /// The final rendered system prompt (cache boundary marker already
    /// stripped — `cache_boundary` below holds the byte offset instead).
    pub text: String,
    /// Byte offset of the cache boundary, if the builder inserted one.
    /// This is the same value that gets threaded into
    /// `ChatRequest::system_prompt_cache_boundary` at runtime.
    pub cache_boundary: Option<usize>,
    /// Every tool that made it into the rendered prompt, in order.
    /// Useful for quick assertions in scripts (e.g. "does the
    /// skills_agent dump contain `composio_execute`?").
    pub tool_names: Vec<String>,
    /// Number of `ToolCategory::Skill` tools included in the dump.
    pub skill_tool_count: usize,
}

/// Render and return the system prompt for the requested agent.
///
/// Builds a real tool registry from the loaded [`Config`], loads the
/// full agent-definition registry (built-ins + `agents/*.toml` overrides),
/// resolves the target agent, and runs it through the matching prompt
/// renderer. See the module docs for the full behaviour contract.
pub async fn dump_agent_prompt(options: DumpPromptOptions) -> Result<DumpedPrompt> {
    tracing::debug!(
        agent_id = %options.agent_id,
        skill_filter = ?options.skill_filter,
        "[debug_dump] rendering prompt"
    );

    // ── Load config + workspace path ──────────────────────────────────
    let mut config = Config::load_or_init()
        .await
        .context("loading Config for prompt dump")?;
    config.apply_env_overrides();

    if let Some(ref override_dir) = options.workspace_dir_override {
        config.workspace_dir = override_dir.clone();
    }
    let workspace_dir = config.workspace_dir.clone();
    std::fs::create_dir_all(&workspace_dir).ok();

    let model_name = options.model_override.clone().unwrap_or_else(|| {
        config
            .default_model
            .clone()
            .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string())
    });

    tracing::debug!(
        workspace_dir = %workspace_dir.display(),
        model = %model_name,
        composio_enabled = config.composio.enabled,
        "[debug_dump] resolved environment"
    );

    // ── Build a real tool registry ─────────────────────────────────────
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

    let composio_key = if config.composio.enabled {
        config.composio.api_key.as_deref()
    } else {
        None
    };
    let composio_entity_id = if config.composio.enabled {
        Some(config.composio.entity_id.as_str())
    } else {
        None
    };

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

    // When requested, inject the Composio meta-tool stubs if (and only
    // if) the real registry didn't already bring them in. This lets a
    // dev machine without OAuth credentials dump the *intended* prompt
    // for `skills_agent` — the names, descriptions and schemas are the
    // same bytes a signed-in user would see. See
    // [`DumpPromptOptions::stub_composio`] for the safety contract.
    if options.stub_composio
        && !tools_vec
            .iter()
            .any(|t| t.name().starts_with("composio_"))
    {
        tracing::debug!("[debug_dump] injecting composio meta-tool stubs");
        tools_vec.extend(build_composio_stub_tools());
    }

    tracing::debug!(
        tool_count = tools_vec.len(),
        "[debug_dump] assembled tool registry"
    );

    // ── Main agent path ────────────────────────────────────────────────
    if options.agent_id == "main" || options.agent_id == "orchestrator_main" {
        return render_main_agent_dump(&workspace_dir, &model_name, &tools_vec);
    }

    // ── Sub-agent path ────────────────────────────────────────────────
    let registry = AgentDefinitionRegistry::load(&workspace_dir)
        .context("loading agent definition registry for prompt dump")?;
    let definition = registry
        .get(&options.agent_id)
        .cloned()
        .ok_or_else(|| {
            let known: Vec<&str> = registry.list().iter().map(|d| d.id.as_str()).collect();
            anyhow!(
                "unknown agent id `{}`. Known agents: [{}] — or pass `main` for the orchestrator prompt.",
                options.agent_id,
                known.join(", ")
            )
        })?;

    render_subagent_dump(
        &definition,
        &workspace_dir,
        &model_name,
        &tools_vec,
        options.skill_filter.as_deref(),
    )
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Build the five Composio meta-tools with a dummy client wired to
/// `http://127.0.0.1:0`. Rendering only calls `name()`, `description()`,
/// and `parameters_schema()` — all of which are static, pure accessors
/// — so the dummy backend URL is never contacted. **Do not** actually
/// execute these tools: calling `.execute()` on a stub would try to
/// POST against the dummy URL.
///
/// This is only used by [`dump_agent_prompt`] when
/// [`DumpPromptOptions::stub_composio`] is `true`, to let prompt
/// engineers inspect the skills_agent prompt on an unauthed machine.
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

fn render_main_agent_dump(
    workspace_dir: &Path,
    model_name: &str,
    tools_vec: &[Box<dyn Tool>],
) -> Result<DumpedPrompt> {
    let prompt_tools = PromptTool::from_tools(tools_vec);
    // Main agent dumps do not apply a visible-tool filter — every
    // tool the registry emits is candidate for rendering.
    let empty_filter: HashSet<String> = HashSet::new();

    // Hydrate the same user-memory blob the runtime would inject on the
    // first turn. The dump intentionally bypasses `Agent::fetch_learned_context`
    // (which needs a live `Memory` backend and the `learning_enabled`
    // flag), but the tree-summarizer side is pure filesystem reads, so
    // we can mirror the runtime path byte-for-byte. This is what makes
    // `openhuman agent dump-prompt --agent main` show the user memory
    // section instead of an empty placeholder when summaries exist on
    // disk.
    let tree_root_summaries =
        crate::openhuman::tree_summarizer::store::collect_root_summaries_with_caps(
            workspace_dir,
            USER_MEMORY_PER_NAMESPACE_MAX_CHARS,
            USER_MEMORY_TOTAL_MAX_CHARS,
        );
    tracing::debug!(
        namespace_count = tree_root_summaries.len(),
        "[debug_dump] hydrated user memory from tree summarizer"
    );
    let learned = LearnedContextData {
        tree_root_summaries,
        ..Default::default()
    };

    let ctx = PromptContext {
        workspace_dir,
        model_name,
        tools: &prompt_tools,
        skills: &[],
        dispatcher_instructions: "",
        learned,
        visible_tool_names: &empty_filter,
    };

    let rendered = SystemPromptBuilder::with_defaults()
        .build_with_cache_metadata(&ctx)
        .context("building main-agent prompt")?;

    let tool_names: Vec<String> = tools_vec.iter().map(|t| t.name().to_string()).collect();
    let skill_tool_count = tools_vec
        .iter()
        .filter(|t| t.category() == ToolCategory::Skill)
        .count();

    Ok(DumpedPrompt {
        agent_id: "main".into(),
        mode: "main",
        model: model_name.to_string(),
        workspace_dir: workspace_dir.to_path_buf(),
        text: rendered.text,
        cache_boundary: rendered.cache_boundary,
        tool_names,
        skill_tool_count,
    })
}

fn render_subagent_dump(
    definition: &AgentDefinition,
    workspace_dir: &Path,
    model_name: &str,
    tools_vec: &[Box<dyn Tool>],
    skill_filter_override: Option<&str>,
) -> Result<DumpedPrompt> {
    // Resolve the archetype prompt body. Inline sources short-circuit
    // immediately; file sources walk the workspace override directory
    // first, mirroring `subagent_runner::load_prompt_source`.
    let archetype_body = match &definition.system_prompt {
        PromptSource::Inline(body) => body.clone(),
        PromptSource::File { path } => {
            let workspace_path = workspace_dir.join("agent").join("prompts").join(path);
            if workspace_path.is_file() {
                std::fs::read_to_string(&workspace_path).with_context(|| {
                    format!("reading archetype prompt at {}", workspace_path.display())
                })?
            } else {
                let workspace_root_path = workspace_dir.join(path);
                if workspace_root_path.is_file() {
                    std::fs::read_to_string(&workspace_root_path).with_context(|| {
                        format!(
                            "reading archetype prompt at {}",
                            workspace_root_path.display()
                        )
                    })?
                } else {
                    tracing::warn!(
                        path = %path,
                        "[debug_dump] archetype prompt file not found, using empty body"
                    );
                    String::new()
                }
            }
        }
    };

    let model = definition.model.resolve(model_name);
    let effective_skill_filter = skill_filter_override.or(definition.skill_filter.as_deref());

    // Apply exactly the same filter the real runner uses so the dump
    // reflects what the sub-agent actually sees.
    let allowed_indices = filter_tool_indices_for_dump(
        tools_vec,
        &definition.tools,
        &definition.disallowed_tools,
        effective_skill_filter,
        definition.category_filter,
    );

    let options = SubagentRenderOptions::from_definition_flags(
        definition.omit_identity,
        definition.omit_safety_preamble,
        definition.omit_skills_catalog,
    );

    let raw = render_subagent_system_prompt(
        workspace_dir,
        &model,
        &allowed_indices,
        tools_vec,
        &archetype_body,
        options,
    );
    let rendered = extract_cache_boundary(&raw);

    let tool_names: Vec<String> = allowed_indices
        .iter()
        .map(|&i| tools_vec[i].name().to_string())
        .collect();
    let skill_tool_count = allowed_indices
        .iter()
        .filter(|&&i| tools_vec[i].category() == ToolCategory::Skill)
        .count();

    tracing::debug!(
        agent_id = %definition.id,
        tool_count = tool_names.len(),
        skill_tool_count,
        cache_boundary = ?rendered.cache_boundary,
        "[debug_dump] sub-agent render complete"
    );

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

/// Standalone copy of the filter logic in
/// [`crate::openhuman::agent::harness::subagent_runner`] so this debug
/// module does not depend on crate-private items. Kept in lockstep with
/// the real `filter_tool_indices` — if you change the order or semantics
/// there, update this function too (and the unit tests below).
fn filter_tool_indices_for_dump(
    parent_tools: &[Box<dyn Tool>],
    scope: &ToolScope,
    disallowed: &[String],
    skill_filter: Option<&str>,
    category_filter: Option<ToolCategory>,
) -> Vec<usize> {
    let disallow_set: HashSet<&str> = disallowed.iter().map(|s| s.as_str()).collect();
    let skill_prefix = skill_filter.map(|s| format!("{s}__"));

    parent_tools
        .iter()
        .enumerate()
        .filter(|(_, tool)| {
            let name = tool.name();
            if disallow_set.contains(name) {
                return false;
            }
            if let Some(required) = category_filter {
                if tool.category() != required {
                    return false;
                }
            }
            if let Some(prefix) = skill_prefix.as_deref() {
                if !name.starts_with(prefix) {
                    return false;
                }
            }
            match scope {
                ToolScope::Wildcard => true,
                ToolScope::Named(allowed) => allowed.iter().any(|n| n == name),
            }
        })
        .map(|(i, _)| i)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::{
        DefinitionSource, ModelSpec, PromptSource, SandboxMode,
    };
    use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
    use async_trait::async_trait;

    /// Minimal tool stub with a configurable category — enough for the
    /// filter/render unit tests below.
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

    fn skills_agent_def() -> AgentDefinition {
        AgentDefinition {
            id: "skills_agent".into(),
            when_to_use: "t".into(),
            display_name: None,
            system_prompt: PromptSource::Inline(
                "# Skills Agent\n\nYou execute skill-category tools.".into(),
            ),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: false,
            omit_skills_catalog: true,
            model: ModelSpec::Inherit,
            temperature: 0.4,
            tools: ToolScope::Wildcard,
            disallowed_tools: vec![],
            skill_filter: None,
            category_filter: Some(ToolCategory::Skill),
            max_iterations: 8,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
            uses_fork_context: false,
            source: DefinitionSource::Builtin,
        }
    }

    #[test]
    fn filter_respects_category_filter() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool {
                name: "shell",
                category: ToolCategory::System,
            }),
            Box::new(StubTool {
                name: "composio_execute",
                category: ToolCategory::Skill,
            }),
            Box::new(StubTool {
                name: "notion__create_page",
                category: ToolCategory::Skill,
            }),
        ];

        let indices = filter_tool_indices_for_dump(
            &tools,
            &ToolScope::Wildcard,
            &[],
            None,
            Some(ToolCategory::Skill),
        );

        let names: Vec<&str> = indices.iter().map(|&i| tools[i].name()).collect();
        assert_eq!(names, vec!["composio_execute", "notion__create_page"]);
    }

    #[test]
    fn render_skills_agent_dump_contains_composio_tool() {
        // Simulates: `openhuman agent dump-prompt --agent skills_agent`
        // with a stub registry that mirrors what the real Composio
        // integration registers. This guards the end-to-end property
        // the user cares about: skills_agent must see composio tools.
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool {
                name: "shell",
                category: ToolCategory::System,
            }),
            Box::new(StubTool {
                name: "composio_list_toolkits",
                category: ToolCategory::Skill,
            }),
            Box::new(StubTool {
                name: "composio_execute",
                category: ToolCategory::Skill,
            }),
        ];

        let workspace = std::env::temp_dir()
            .join(format!("openhuman_debug_dump_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();

        let definition = skills_agent_def();
        let dumped = render_subagent_dump(
            &definition,
            &workspace,
            "reasoning-v1",
            &tools,
            None,
        )
        .expect("skills_agent prompt should render");

        assert_eq!(dumped.mode, "subagent");
        assert!(
            dumped.tool_names.iter().any(|n| n == "composio_execute"),
            "skills_agent dump missing composio_execute; got: {:?}",
            dumped.tool_names
        );
        assert!(
            !dumped.tool_names.iter().any(|n| n == "shell"),
            "skills_agent dump should not include system tools; got: {:?}",
            dumped.tool_names
        );
        assert!(
            dumped.text.contains("composio_execute"),
            "rendered prompt body missing composio_execute — composio toolkit is not reaching the skills agent"
        );
        assert!(
            dumped.text.contains("## Safety"),
            "skills_agent dump should include the safety preamble (omit_safety_preamble = false)"
        );
        assert_eq!(dumped.skill_tool_count, 2);

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn render_with_skill_filter_narrows_to_one_integration() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool {
                name: "composio_execute",
                category: ToolCategory::Skill,
            }),
            Box::new(StubTool {
                name: "notion__create_page",
                category: ToolCategory::Skill,
            }),
            Box::new(StubTool {
                name: "gmail__send_email",
                category: ToolCategory::Skill,
            }),
        ];

        let workspace = std::env::temp_dir()
            .join(format!("openhuman_debug_dump_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();

        let definition = skills_agent_def();
        let dumped = render_subagent_dump(
            &definition,
            &workspace,
            "reasoning-v1",
            &tools,
            Some("notion"),
        )
        .expect("filtered dump should render");

        assert_eq!(dumped.tool_names, vec!["notion__create_page"]);
        let _ = std::fs::remove_dir_all(workspace);
    }
}
