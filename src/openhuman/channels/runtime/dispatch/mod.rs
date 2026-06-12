//! Channel runtime loop and message processing.
//!
//! Sub-modules:
//! * [`helpers`]   — small stateless helpers (context block, ACK reaction, typing, workers).
//! * [`routing`]   — agent selection and tool-scoping ([`AgentScoping`],
//!   [`resolve_target_agent`], [`build_visible_tool_set`]).
//! * [`processor`] — core message pipeline ([`process_channel_message`],
//!   [`run_message_dispatch_loop`]) and approval-surface gate.

mod helpers;
mod processor;
mod routing;

pub(crate) use processor::{process_channel_message, run_message_dispatch_loop};

// `channel_has_approval_surface` stays pub(crate) on processor; re-export so
// the inline test module can reach it via `super::channel_has_approval_surface`.
#[cfg(test)]
use processor::channel_has_approval_surface;

// Re-export internal helpers accessed by test_support (cfg(any(test,
// debug_assertions))) and the inline #[cfg(test)] modules via `super::*`.
#[cfg(any(test, debug_assertions))]
use helpers::{build_channel_context_block, select_acknowledgment_reaction};

#[cfg(test)]
use helpers::{contains_any, starts_with_any};

#[cfg(test)]
use routing::{build_visible_tool_set, AgentScoping};

#[cfg(any(test, debug_assertions))]
use crate::openhuman::channels::traits;

#[cfg(any(test, debug_assertions))]
pub mod test_support {
    //! Debug-build seams for raw integration coverage of dispatch helpers.

    use super::*;

    pub fn build_channel_context_block_for_test(msg: &traits::ChannelMessage) -> String {
        build_channel_context_block(msg)
    }

    pub fn select_acknowledgment_reaction_for_test(content: &str) -> &'static str {
        select_acknowledgment_reaction(content)
    }
}

#[cfg(test)]
mod scoping_tests {
    //! Pure-function unit tests for the agent-scoping helpers added by
    //! the #525/#526 fix. These exercise the synchronous logic without
    //! touching the real `Config::load_or_init` disk read or the global
    //! `AgentDefinitionRegistry`, so they can run in any environment.
    //!
    //! End-to-end exercise of the dispatch path is covered by the
    //! existing `runtime_dispatch::dispatch_routes_through_agent_run_turn_
    //! bus_handler` integration test, which still passes after the new
    //! fields landed (the resolver gracefully falls back to
    //! `AgentScoping::unscoped()` when no orchestrator is registered in
    //! the test environment).

    use super::*;
    use crate::openhuman::agent::harness::definition::{
        AgentDefinition, DefinitionSource, ModelSpec, PromptSource, SandboxMode, ToolScope,
    };
    use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
    use async_trait::async_trait;

    /// Minimal owned tool stub — just enough for `build_visible_tool_set`
    /// to read its `name()`.
    struct StubTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn category(&self) -> ToolCategory {
            ToolCategory::System
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::None
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
    }

    fn def_with_scope(scope: ToolScope) -> AgentDefinition {
        AgentDefinition {
            id: "test_agent".into(),
            when_to_use: "test".into(),
            display_name: None,
            system_prompt: PromptSource::Inline(String::new()),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            omit_skills_catalog: true,
            omit_profile: true,
            omit_memory_md: true,
            model: ModelSpec::Inherit,
            temperature: 0.4,
            tools: scope,
            disallowed_tools: vec![],
            skill_filter: None,
            extra_tools: vec![],
            max_iterations: 8,
            iteration_policy: Default::default(),
            max_result_chars: None,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
            trigger_memory_agent: Default::default(),
            subagents: vec![],
            delegate_name: None,
            agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
            source: DefinitionSource::Builtin,
        }
    }

    /// `ToolScope::Wildcard` must yield `None` — the prompt builder
    /// treats `None` as "no filter, every tool visible", which is the
    /// correct behaviour for agents like `integrations_agent` that want the
    /// full skill-category catalogue. Even when extras are present, a
    /// wildcard agent should not start filtering.
    #[test]
    fn wildcard_scope_yields_none_filter() {
        let def = def_with_scope(ToolScope::Wildcard);
        let extras: Vec<Box<dyn Tool>> = vec![Box::new(StubTool { name: "research" })];
        assert!(build_visible_tool_set(&def, &extras).is_none());
        assert!(build_visible_tool_set(&def, &[]).is_none());
    }

    /// `ToolScope::Named` with no extras returns exactly the named set.
    /// For agents with a narrow tool scope (e.g. 2 tools in TOML,
    /// no delegation, no extras) → 2 entries in the visibility whitelist.
    #[test]
    fn named_scope_without_extras_returns_named_only() {
        let def = def_with_scope(ToolScope::Named(vec![
            "memory_recall".into(),
            "ask_user_clarification".into(),
        ]));
        let set = build_visible_tool_set(&def, &[]).expect("named scope yields Some");
        assert_eq!(set.len(), 2);
        assert!(set.contains("memory_recall"));
        assert!(set.contains("ask_user_clarification"));
    }

    /// `ToolScope::Named` with extras returns the union of the TOML
    /// named list and the extras' names. This is the orchestrator's
    /// path: direct tools from the TOML + the synthesised delegation
    /// tools (`research`, `plan`, `delegate_to_integrations_agent`)
    /// → all of them visible to the orchestrator's LLM. The stub
    /// names in this test are arbitrary; they exercise the union
    /// logic, not the real synthesiser.
    #[test]
    fn named_scope_with_extras_returns_union() {
        let def = def_with_scope(ToolScope::Named(vec![
            "query_memory".into(),
            "ask_user_clarification".into(),
            "spawn_subagent".into(),
        ]));
        let extras: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool { name: "research" }),
            Box::new(StubTool {
                name: "delegate_gmail",
            }),
            Box::new(StubTool {
                name: "delegate_github",
            }),
        ];
        let set = build_visible_tool_set(&def, &extras).expect("named scope yields Some");
        assert_eq!(set.len(), 6);
        assert!(set.contains("query_memory"));
        assert!(set.contains("ask_user_clarification"));
        assert!(set.contains("spawn_subagent"));
        assert!(set.contains("research"));
        assert!(set.contains("delegate_gmail"));
        assert!(set.contains("delegate_github"));
    }

    /// Empty `Named` list with extras still yields `Some` containing
    /// just the extras — useful for hypothetical agents that only
    /// reach the world via delegation, with no direct tools.
    #[test]
    fn empty_named_with_extras_returns_extras_only() {
        let def = def_with_scope(ToolScope::Named(vec![]));
        let extras: Vec<Box<dyn Tool>> = vec![Box::new(StubTool {
            name: "delegate_only",
        })];
        let set = build_visible_tool_set(&def, &extras).expect("named scope yields Some");
        assert_eq!(set.len(), 1);
        assert!(set.contains("delegate_only"));
    }

    /// Empty `Named` list with no extras yields an empty `Some(set)` —
    /// effectively "no tools visible". The prompt loop's `is_visible`
    /// helper treats `Some(empty)` differently from `None`: the former
    /// means "filter active, nothing matches" so the LLM gets an empty
    /// tool list, while the latter means "no filter at all".
    #[test]
    fn empty_named_with_no_extras_returns_empty_set() {
        let def = def_with_scope(ToolScope::Named(vec![]));
        let set = build_visible_tool_set(&def, &[]).expect("named scope yields Some");
        assert!(set.is_empty());
    }

    /// Duplicate names across named + extras are de-duplicated by the
    /// HashSet — no double-counting if a workspace override happens to
    /// list a delegation tool name in the direct `named` list too.
    #[test]
    fn duplicate_names_across_named_and_extras_are_deduplicated() {
        let def = def_with_scope(ToolScope::Named(vec![
            "research".into(),
            "query_memory".into(),
        ]));
        let extras: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool { name: "research" }), // collides with named
            Box::new(StubTool { name: "plan" }),
        ];
        let set = build_visible_tool_set(&def, &extras).expect("named scope yields Some");
        assert_eq!(set.len(), 3);
        assert!(set.contains("research"));
        assert!(set.contains("query_memory"));
        assert!(set.contains("plan"));
    }

    /// `AgentScoping::unscoped` is the safe-fallback constructor used
    /// when the registry is uninitialised or the target agent isn't
    /// found. All three fields must default to "no scoping applied"
    /// so the channel turn runs with the legacy unfiltered behaviour.
    #[test]
    fn agent_scoping_unscoped_has_no_filter_or_extras() {
        let scoping = AgentScoping::unscoped();
        assert!(scoping.target_agent_id.is_none());
        assert!(scoping.visible_tool_names.is_none());
        assert!(scoping.extra_tools.is_empty());
    }
}

#[cfg(test)]
mod approval_surface_gating_tests {
    use super::channel_has_approval_surface;

    // Sub-issue 2 of #3098: this gate is what decides whether the dispatch
    // loop sets an `ApprovalChatContext` (→ gate fires for `Prompt`-class
    // tools) versus the legacy bypass (→ tool calls silently allowed).
    // Pin the matrix so silently broadening to a new channel can't
    // accidentally TTL-deny every parked tool call there.

    #[test]
    fn telegram_has_approval_surface() {
        assert!(channel_has_approval_surface("telegram"));
    }

    #[test]
    fn other_channels_do_not_yet_have_an_approval_surface() {
        for channel in ["discord", "slack", "imessage", "mattermost", "web", "irc"] {
            assert!(
                !channel_has_approval_surface(channel),
                "channel {channel:?} is not (yet) wired to a per-channel approval surface; \
                 the dispatch loop must not scope an ApprovalChatContext for it or every \
                 Prompt-class tool call will park with nobody to answer and TTL-deny"
            );
        }
    }

    #[test]
    fn unknown_channel_does_not_have_approval_surface() {
        assert!(!channel_has_approval_surface(""));
        assert!(!channel_has_approval_surface("Telegram")); // case-sensitive on purpose
        assert!(!channel_has_approval_surface("telegram-bot"));
    }
}

#[cfg(test)]
#[path = "../dispatch_tests.rs"]
mod tests;
