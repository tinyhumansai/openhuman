//! Built-in [`AgentDefinition`]s derived from [`AgentArchetype`].
//!
//! These cover the eight historical archetypes plus a synthetic `fork`
//! definition that the runner uses for byte-exact prompt-cache reuse.
//! Custom YAML definitions loaded later override any built-in with the
//! same id.

use super::archetypes::AgentArchetype;
use super::definition::{
    AgentDefinition, DefinitionSource, ModelSpec, PromptSource, SandboxMode, ToolScope,
};

/// All built-in definitions, in stable order.
pub fn all() -> Vec<AgentDefinition> {
    let mut out: Vec<AgentDefinition> = AgentArchetype::all()
        .iter()
        .map(|arch| from_archetype(*arch))
        .collect();
    out.push(fork_definition());
    out
}

/// Construct an [`AgentDefinition`] for one [`AgentArchetype`].
///
/// Reads `default_model_hint`, `allowed_tools`, `default_max_iterations`,
/// `sandbox_mode`, and `system_prompt_path` from the existing archetype
/// metadata so this stays a single source of truth.
pub fn from_archetype(arch: AgentArchetype) -> AgentDefinition {
    let id = arch.to_string();
    let when_to_use = when_to_use_for(arch).to_string();
    let display_name = Some(display_name_for(arch).to_string());
    let system_prompt = PromptSource::File {
        path: arch.system_prompt_path().to_string(),
    };

    let tools = match arch.allowed_tools() {
        // SkillsAgent: dynamic — at spawn time the runner picks up all
        // currently-loaded skill tools (filtered by `skill_filter` if set).
        None => ToolScope::Wildcard,
        Some(allowed) => ToolScope::Named(allowed.iter().map(|s| (*s).to_string()).collect()),
    };

    // SkillsAgent's default skill filter is None — meaning "all skill tools".
    // Per-API specialists (Notion, Gmail, …) are layered on top by setting
    // `skill_filter` either in YAML (custom definition) or as a per-spawn arg.
    let skill_filter = None;

    // Sub-agents always run with the cheaper, narrower archetype model
    // hint. Use `ModelSpec::Inherit` if you want them to share the parent's
    // pinned model — see the `fork` synthetic definition below.
    let model = ModelSpec::Hint(arch.default_model_hint().to_string());

    let sandbox_mode = match arch.sandbox_mode() {
        "sandboxed" => SandboxMode::Sandboxed,
        "read_only" => SandboxMode::ReadOnly,
        _ => SandboxMode::None,
    };

    // Code executor / tool maker / skills agent need the safety preamble
    // (they actually touch the world). Pure read-only roles strip it.
    let omit_safety_preamble = !matches!(
        arch,
        AgentArchetype::CodeExecutor | AgentArchetype::ToolMaker | AgentArchetype::SkillsAgent
    );

    AgentDefinition {
        id,
        when_to_use,
        display_name,
        system_prompt,
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble,
        omit_skills_catalog: true,
        model,
        temperature: 0.4,
        tools,
        disallowed_tools: vec![],
        skill_filter,
        max_iterations: arch.default_max_iterations(),
        timeout_secs: None,
        sandbox_mode,
        background: arch.is_background(),
        uses_fork_context: false,
        source: DefinitionSource::Builtin,
    }
}

/// The synthetic `fork` definition. Tells the runner to bypass normal
/// prompt construction and replay the parent's exact rendered system
/// prompt + tool schemas + message prefix from
/// [`super::fork_context::ForkContext`]. The OpenAI-compatible backend's
/// automatic prefix caching turns this into a real token win.
pub fn fork_definition() -> AgentDefinition {
    AgentDefinition {
        id: "fork".into(),
        when_to_use: "Spawn a parallel sub-task that shares the parent's full system \
                      prompt, tool set, and message history byte-for-byte. Use when \
                      decomposing a task into independent parallel work streams that \
                      benefit from prefix-cache reuse on the inference backend."
            .into(),
        display_name: Some("Fork".into()),
        // Prompt source is irrelevant — the runner reads from ForkContext.
        // We still set a placeholder so YAML round-trips work.
        system_prompt: PromptSource::Inline(String::new()),
        // Fork preserves bytes — DO NOT strip anything from the parent's prompt.
        omit_identity: false,
        omit_memory_context: false,
        omit_safety_preamble: false,
        omit_skills_catalog: false,
        model: ModelSpec::Inherit,
        // Inherit the parent's temperature too — set to a sentinel that the
        // runner replaces with the parent's actual temp at spawn time.
        // (We use 0.7 here as a safe default for documentation; the runner
        // overrides it from `ParentExecutionContext::temperature`.)
        temperature: 0.7,
        tools: ToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        // Fork inherits the parent's max iterations from the runtime.
        max_iterations: 15,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        uses_fork_context: true,
        source: DefinitionSource::Builtin,
    }
}

fn when_to_use_for(arch: AgentArchetype) -> &'static str {
    match arch {
        AgentArchetype::Orchestrator => {
            "Staff Engineer — routes, judges quality, synthesises. Never writes code itself. \
             You should not normally spawn another orchestrator from inside one."
        }
        AgentArchetype::Planner => {
            "Architect — break a complex task into a small DAG of subtasks with \
             explicit acceptance criteria. Read-only; produces JSON, not code."
        }
        AgentArchetype::CodeExecutor => {
            "Sandboxed developer — writes, runs, and debugs code until tests pass. \
             Use for any task that requires producing or modifying source files \
             and exercising them with shell or test commands."
        }
        AgentArchetype::SkillsAgent => {
            "Skill tool specialist — executes installed QuickJS skill tools \
             (Notion, Gmail, …). Use when the task should be completed via a \
             user-installed skill rather than raw HTTP/file I/O. Pair with a \
             `skill_filter` argument to scope to a single skill."
        }
        AgentArchetype::ToolMaker => {
            "Self-healer — writes a polyfill script when a required command is \
             missing on the host. Very narrow scope; max 2 iterations."
        }
        AgentArchetype::Researcher => {
            "Web & docs crawler — reads real documentation, compresses to dense \
             markdown. Use for any task that requires looking up external knowledge."
        }
        AgentArchetype::Critic => {
            "Adversarial reviewer — reviews diffs and code against project rules, \
             flags vulnerabilities, regressions, and missing tests. Read-only."
        }
        AgentArchetype::Archivist => {
            "Background librarian — extracts lessons from a completed session, \
             updates MEMORY.md, and indexes to FTS5. Runs cheap and slow."
        }
    }
}

fn display_name_for(arch: AgentArchetype) -> &'static str {
    match arch {
        AgentArchetype::Orchestrator => "Orchestrator",
        AgentArchetype::Planner => "Planner",
        AgentArchetype::CodeExecutor => "Code Executor",
        AgentArchetype::SkillsAgent => "Skills Agent",
        AgentArchetype::ToolMaker => "Tool Maker",
        AgentArchetype::Researcher => "Researcher",
        AgentArchetype::Critic => "Critic",
        AgentArchetype::Archivist => "Archivist",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_definitions_present() {
        let defs = all();
        // 8 archetypes + 1 synthetic `fork`.
        assert_eq!(defs.len(), 9);
    }

    #[test]
    fn each_archetype_yields_an_id() {
        for arch in AgentArchetype::all() {
            let def = from_archetype(*arch);
            assert_eq!(def.id, arch.to_string());
            assert!(!def.when_to_use.is_empty());
            assert_eq!(def.source, DefinitionSource::Builtin);
        }
    }

    #[test]
    fn code_executor_keeps_safety_preamble() {
        let def = from_archetype(AgentArchetype::CodeExecutor);
        assert!(!def.omit_safety_preamble);
    }

    #[test]
    fn critic_strips_safety_preamble() {
        let def = from_archetype(AgentArchetype::Critic);
        assert!(def.omit_safety_preamble);
    }

    #[test]
    fn skills_agent_uses_wildcard_tools() {
        let def = from_archetype(AgentArchetype::SkillsAgent);
        assert!(matches!(def.tools, ToolScope::Wildcard));
    }

    #[test]
    fn code_executor_uses_named_tools() {
        let def = from_archetype(AgentArchetype::CodeExecutor);
        match def.tools {
            ToolScope::Named(tools) => {
                assert!(tools.iter().any(|t| t == "shell"));
                assert!(tools.iter().any(|t| t == "file_write"));
            }
            ToolScope::Wildcard => panic!("expected named tools for code_executor"),
        }
    }

    #[test]
    fn fork_definition_has_uses_fork_context_true() {
        let def = fork_definition();
        assert_eq!(def.id, "fork");
        assert!(def.uses_fork_context);
        assert!(matches!(def.model, ModelSpec::Inherit));
        assert!(matches!(def.tools, ToolScope::Wildcard));
        // Fork preserves bytes — must NOT strip anything.
        assert!(!def.omit_identity);
        assert!(!def.omit_memory_context);
        assert!(!def.omit_safety_preamble);
        assert!(!def.omit_skills_catalog);
    }

    #[test]
    fn archetype_max_iterations_is_propagated() {
        let critic = from_archetype(AgentArchetype::Critic);
        assert_eq!(critic.max_iterations, 5);
        let tool_maker = from_archetype(AgentArchetype::ToolMaker);
        assert_eq!(tool_maker.max_iterations, 2);
    }

    #[test]
    fn sandbox_modes_map_correctly() {
        assert_eq!(
            from_archetype(AgentArchetype::CodeExecutor).sandbox_mode,
            SandboxMode::Sandboxed
        );
        assert_eq!(
            from_archetype(AgentArchetype::Critic).sandbox_mode,
            SandboxMode::ReadOnly
        );
        assert_eq!(
            from_archetype(AgentArchetype::Researcher).sandbox_mode,
            SandboxMode::None
        );
    }
}
