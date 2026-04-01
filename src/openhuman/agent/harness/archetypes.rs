//! Agent archetypes — the 8 specialized roles in the multi-agent harness.
//!
//! Each archetype defines a default model tier, tool whitelist, sandbox mode,
//! max iterations, and system prompt path. The Orchestrator uses these to
//! construct ephemeral sub-agents via `AgentBuilder`.

use serde::{Deserialize, Serialize};

/// The 8 specialised agent roles in the multi-agent topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentArchetype {
    /// Staff Engineer — routes, judges quality, synthesises. Never writes code.
    Orchestrator,
    /// Architect — breaks complex tasks into a DAG of subtasks with acceptance criteria.
    Planner,
    /// Sandboxed developer — writes/runs code, fixes bugs until tests pass.
    CodeExecutor,
    /// Skill tool specialist — executes QuickJS skill tools (Notion, Gmail, …).
    SkillsAgent,
    /// Self-healer — writes polyfill scripts when a required command is missing.
    ToolMaker,
    /// Web & doc crawler — reads real documentation, compresses to dense markdown.
    Researcher,
    /// Adversarial reviewer — reviews diffs against SOUL.md, flags vulnerabilities.
    Critic,
    /// Background librarian — extracts lessons, updates MEMORY.md, indexes to FTS5.
    Archivist,
}

impl std::fmt::Display for AgentArchetype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Orchestrator => "orchestrator",
            Self::Planner => "planner",
            Self::CodeExecutor => "code_executor",
            Self::SkillsAgent => "skills_agent",
            Self::ToolMaker => "tool_maker",
            Self::Researcher => "researcher",
            Self::Critic => "critic",
            Self::Archivist => "archivist",
        };
        write!(f, "{name}")
    }
}

impl AgentArchetype {
    /// Model hint passed to `RouterProvider` (prefixed with `"hint:"` at call site).
    pub fn default_model_hint(&self) -> &'static str {
        match self {
            Self::Orchestrator => "reasoning",
            Self::Planner => "reasoning",
            Self::CodeExecutor => "coding",
            Self::SkillsAgent => "agentic",
            Self::ToolMaker => "coding",
            Self::Researcher => "agentic",
            Self::Critic => "reasoning",
            // Archivist uses the cheapest available model (local preferred).
            Self::Archivist => "local",
        }
    }

    /// Static tool-name whitelist for this archetype.
    ///
    /// Returns `None` for `SkillsAgent` because its tools are dynamic
    /// (populated from `RuntimeEngine::all_tools()` at construction time).
    pub fn allowed_tools(&self) -> Option<&'static [&'static str]> {
        match self {
            Self::Orchestrator => Some(&[
                "spawn_subagent",
                "query_memory",
                "read_workspace_state",
                "ask_user_clarification",
            ]),
            Self::Planner => Some(&["query_memory", "read_workspace_state", "file_read"]),
            Self::CodeExecutor => Some(&["shell", "file_read", "file_write", "git_operations"]),
            Self::SkillsAgent => None, // dynamic — all skill-registered tools + memory_recall
            Self::ToolMaker => Some(&["file_write", "shell"]),
            Self::Researcher => Some(&["http_request", "web_search", "file_read", "memory_recall"]),
            Self::Critic => Some(&["read_diff", "run_linter", "run_tests", "file_read"]),
            Self::Archivist => Some(&["update_memory_md", "insert_sql_record", "memory_store"]),
        }
    }

    /// Maximum tool-call iterations for a single turn.
    pub fn default_max_iterations(&self) -> usize {
        match self {
            Self::Orchestrator => 15,
            Self::Planner => 5,
            Self::CodeExecutor => 10,
            Self::SkillsAgent => 10,
            Self::ToolMaker => 2,
            Self::Researcher => 8,
            Self::Critic => 5,
            Self::Archivist => 3,
        }
    }

    /// Sandbox mode identifier consumed by `SecurityPolicy` construction.
    ///
    /// * `"sandboxed"` — drop-sudo, restricted filesystem (Landlock / Bubblewrap).
    /// * `"read_only"` — only read operations allowed.
    /// * `"none"` — application-layer validation only.
    pub fn sandbox_mode(&self) -> &'static str {
        match self {
            Self::CodeExecutor | Self::ToolMaker => "sandboxed",
            Self::Critic | Self::Planner => "read_only",
            _ => "none",
        }
    }

    /// Relative path (from `agent/prompts/`) to the archetype's system prompt.
    pub fn system_prompt_path(&self) -> &'static str {
        match self {
            Self::Orchestrator => "ORCHESTRATOR.md",
            Self::Planner => "PLANNER.md",
            Self::CodeExecutor => "archetypes/code_executor.md",
            Self::SkillsAgent => "archetypes/skills_agent.md",
            Self::ToolMaker => "archetypes/code_executor.md", // shares with CodeExecutor
            Self::Researcher => "archetypes/researcher.md",
            Self::Critic => "archetypes/critic.md",
            Self::Archivist => "archetypes/archivist.md",
        }
    }

    /// Whether this archetype runs as a background task (fire-and-forget).
    pub fn is_background(&self) -> bool {
        matches!(self, Self::Archivist)
    }

    /// Whether this archetype should receive learning hooks.
    pub fn has_learning_hooks(&self) -> bool {
        matches!(self, Self::Orchestrator | Self::Archivist)
    }

    /// All archetype variants (useful for iteration / config defaults).
    pub fn all() -> &'static [AgentArchetype] {
        &[
            Self::Orchestrator,
            Self::Planner,
            Self::CodeExecutor,
            Self::SkillsAgent,
            Self::ToolMaker,
            Self::Researcher,
            Self::Critic,
            Self::Archivist,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_archetypes_have_model_hints() {
        for arch in AgentArchetype::all() {
            let hint = arch.default_model_hint();
            assert!(!hint.is_empty(), "{arch} has empty model hint");
        }
    }

    #[test]
    fn skills_agent_has_dynamic_tools() {
        assert!(AgentArchetype::SkillsAgent.allowed_tools().is_none());
    }

    #[test]
    fn code_executor_is_sandboxed() {
        assert_eq!(AgentArchetype::CodeExecutor.sandbox_mode(), "sandboxed");
        assert_eq!(AgentArchetype::ToolMaker.sandbox_mode(), "sandboxed");
    }

    #[test]
    fn critic_is_read_only() {
        assert_eq!(AgentArchetype::Critic.sandbox_mode(), "read_only");
    }

    #[test]
    fn orchestrator_never_has_code_tools() {
        let tools = AgentArchetype::Orchestrator.allowed_tools().unwrap();
        assert!(!tools.contains(&"shell"));
        assert!(!tools.contains(&"file_write"));
    }

    #[test]
    fn display_roundtrip() {
        for arch in AgentArchetype::all() {
            let s = arch.to_string();
            assert!(!s.is_empty());
        }
    }
}
