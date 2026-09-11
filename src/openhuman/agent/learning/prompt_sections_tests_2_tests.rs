use super::*;
use crate::openhuman::agent::context::prompt::LearnedContextData;
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

struct NoopMemory;

#[async_trait]
impl Memory for NoopMemory {
    fn name(&self) -> &str {
        "noop"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: crate::openhuman::memory::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(
        &self,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

fn prompt_context(learned: LearnedContextData) -> PromptContext<'static> {
    let visible_tool_names = Box::leak(Box::new(HashSet::new()));
    PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        learned,
        visible_tool_names,
        tool_call_format: crate::openhuman::agent::context::prompt::ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
        curated_snapshot: None,
    }
}

#[test]
fn learned_context_section_renders_observations_and_patterns() {
    let section = LearnedContextSection::new(Arc::new(NoopMemory));
    let rendered = section
        .build(&prompt_context(LearnedContextData {
            observations: vec!["Tool use succeeded".into()],
            patterns: vec!["User prefers terse replies".into()],
            user_profile: Vec::new(),
            reflections: Vec::new(),
            tree_root_summaries: Vec::new(),
        }))
        .unwrap();

    assert_eq!(section.name(), "learned_context");
    assert!(rendered.contains("## Learned Context"));
    assert!(rendered.contains("### Recent Observations"));
    assert!(rendered.contains("- Tool use succeeded"));
    assert!(rendered.contains("### Recognized Patterns"));
    assert!(rendered.contains("- User prefers terse replies"));
}

#[test]
fn learned_context_section_returns_empty_without_entries() {
    let section = LearnedContextSection::new(Arc::new(NoopMemory));
    assert!(section
        .build(&prompt_context(LearnedContextData::default()))
        .unwrap()
        .is_empty());
}

#[test]
fn user_profile_section_renders_bullets() {
    let section = UserProfileSection::new(Arc::new(NoopMemory));
    let rendered = section
        .build(&prompt_context(LearnedContextData {
            observations: Vec::new(),
            patterns: Vec::new(),
            user_profile: vec![
                "Timezone: America/Los_Angeles".into(),
                "Prefers Rust".into(),
            ],
            reflections: Vec::new(),
            tree_root_summaries: Vec::new(),
        }))
        .unwrap();

    assert_eq!(section.name(), "user_profile");
    assert!(rendered.starts_with("## Your standing preferences\n\n"));
    assert!(rendered.contains("- Timezone: America/Los_Angeles"));
    assert!(rendered.contains("- Prefers Rust"));
}

#[test]
fn user_profile_section_returns_empty_without_profile_entries() {
    let section = UserProfileSection::new(Arc::new(NoopMemory));
    assert!(section
        .build(&prompt_context(LearnedContextData::default()))
        .unwrap()
        .is_empty());
}

// ── load_learned_from_cache ───────────────────────────────────────────────

#[tokio::test]
async fn load_learned_from_cache_formats_active_facets() {
    use tinymemory_api::provider::{FacetState, FacetType, ProfileFacet, UserState};
    let cache = crate::openhuman::agent::learning::test_profile::in_memory_cache();

    let make_facet = |id: &str, key: &str, value: &str, stab: f64| ProfileFacet {
        facet_id: id.into(),
        facet_type: FacetType::Preference,
        key: key.into(),
        value: value.into(),
        confidence: 0.8,
        evidence_count: 2,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1200.0,
        state: FacetState::Active,
        stability: stab,
        user_state: UserState::Auto,
        evidence_refs: vec![],
        class: None,
        cue_families: None,
    };

    cache
        .upsert(&make_facet("f1", "style/verbosity", "terse", 2.0))
        .await
        .unwrap();
    cache
        .upsert(&make_facet("f2", "identity/name", "Alice", 1.8))
        .await
        .unwrap();
    cache
        .upsert(&make_facet(
            "f3",
            "goal/learn_rust",
            "Learn Rust this year",
            1.6,
        ))
        .await
        .unwrap();

    // Provisional — should NOT appear.
    let mut prov = make_facet("f4", "style/tone", "formal", 0.8);
    prov.state = FacetState::Provisional;
    cache.upsert(&prov).await.unwrap();

    let result = load_learned_from_cache(&cache).await;

    assert!(
        !result.is_empty(),
        "should produce entries for Active facets"
    );
    // Phase 4 format: "**style/verbosity**: terse"
    assert!(
        result.iter().any(|s| s.contains("style/verbosity")),
        "style/verbosity should appear"
    );
    assert!(
        result
            .iter()
            .any(|s| s.contains("**style/verbosity**: terse")),
        "style/verbosity should use Phase 4 bold format"
    );
    // Goal class → value only (no key prefix)
    assert!(
        result.iter().any(|s| s == "Learn Rust this year"),
        "goal class should render value only"
    );
    // Provisional should not appear
    assert!(
        !result.iter().any(|s| s.contains("style/tone")),
        "provisional facet must not appear in cache prompt"
    );
}

#[tokio::test]
async fn load_learned_from_cache_empty_when_no_active_facets() {
    let cache = crate::openhuman::agent::learning::test_profile::in_memory_cache();

    let result = load_learned_from_cache(&cache).await;
    assert!(result.is_empty());
}

// ── MemoryAccessSection ───────────────────────────────────────────────────

#[test]
fn memory_access_section_renders_static_text() {
    let section = MemoryAccessSection;
    assert_eq!(section.name(), "memory_access");
    let rendered = section
        .build(&prompt_context(LearnedContextData::default()))
        .unwrap();
    assert!(
        rendered.contains("## Memory access"),
        "heading missing:\n{rendered}"
    );
    assert!(
        rendered.contains("memory_recall"),
        "memory_recall tool not mentioned:\n{rendered}"
    );
    assert!(
        rendered.contains("memory_search"),
        "memory_search tool not mentioned:\n{rendered}"
    );
    // Verify the rendered text matches the constant.
    assert_eq!(rendered.trim(), MEMORY_ACCESS_INSTRUCTION.trim());
}

#[test]
fn memory_access_section_present_in_system_prompt_compose() {
    // Verify the section renders correctly when added to a prompt composition.
    let section = MemoryAccessSection;
    let rendered = section
        .build(&prompt_context(LearnedContextData::default()))
        .unwrap();
    // Spot-check the content constraint: ≤ 80 tokens (rough word count).
    let word_count = rendered.split_whitespace().count();
    assert!(
        word_count <= 100,
        "MemoryAccessSection is too long ({word_count} words, target ≤ 80 tokens)"
    );
    // The section name must be stable (used for insert_section_before).
    assert_eq!(section.name(), "memory_access");
    // Content check: the section must mention both retrieval tools.
    assert!(rendered.contains("memory_recall"));
    assert!(rendered.contains("memory_search"));
    // Verify it is non-empty for any PromptContext (not context-gated).
    let empty_ctx = prompt_context(LearnedContextData::default());
    let rendered_empty_ctx = section.build(&empty_ctx).unwrap();
    assert!(
        !rendered_empty_ctx.trim().is_empty(),
        "must render regardless of learned context"
    );
}

// ── MemoryWriteSection + tool gating (#6048) ─────────────────────────────────

/// A tool that is nothing but a name: the gate reads names only.
struct NamedTool(&'static str);

#[async_trait]
impl Tool for NamedTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "stub"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> anyhow::Result<crate::openhuman::tools::traits::ToolResult> {
        Ok(crate::openhuman::tools::traits::ToolResult::success(
            String::new(),
        ))
    }
}

fn named(names: &[&'static str]) -> Vec<Box<dyn Tool>> {
    names
        .iter()
        .map(|name| Box::new(NamedTool(name)) as Box<dyn Tool>)
        .collect()
}

fn visible(names: &[&str]) -> HashSet<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

#[test]
fn memory_write_section_states_the_rule_the_bug_needed() {
    let section = MemoryWriteSection::new(true, true, false);
    assert_eq!(section.name(), "memory_write");
    let rendered = section
        .build(&prompt_context(LearnedContextData::default()))
        .unwrap();
    assert_eq!(
        rendered.trim(),
        memory_write_instruction(true, true, false).trim()
    );
    assert!(rendered.contains("## Remembering"), "{rendered}");
    assert!(
        rendered.contains("Never say saved"),
        "the instruction must forbid claiming a save that did not happen: {rendered}"
    );
    // Not context-gated: it renders for an empty learned context too.
    let empty = MemoryWriteSection::new(true, true, false)
        .build(&prompt_context(LearnedContextData::default()))
        .unwrap();
    assert!(!empty.trim().is_empty());
}

/// The section names the routes it was given and no others. Telling a session
/// that holds one write tool to call the other is the same class of mistake as
/// registering the section with no write tool at all.
#[test]
fn memory_write_instruction_names_only_the_offered_tools() {
    let both = memory_write_instruction(true, true, false);
    assert!(
        both.contains("`save_preference`") && both.contains("`memory_store`"),
        "{both}"
    );

    let preferences_only = memory_write_instruction(true, false, false);
    assert!(
        preferences_only.contains("`save_preference`"),
        "{preferences_only}"
    );
    assert!(
        !preferences_only.contains("`memory_store`"),
        "a session without memory_store must not be sent to it: {preferences_only}"
    );

    let facts_only = memory_write_instruction(false, true, false);
    assert!(facts_only.contains("`memory_store`"), "{facts_only}");
    assert!(
        !facts_only.contains("`save_preference`"),
        "a session without save_preference must not be sent to it: {facts_only}"
    );

    // Every variant still carries the heading, the rule, and the word ceiling.
    for rendered in [&both, &preferences_only, &facts_only] {
        assert!(rendered.contains("## Remembering"), "{rendered}");
        assert!(rendered.contains("Never say saved"), "{rendered}");
        let words = rendered.split_whitespace().count();
        assert!(
            words <= 80,
            "instruction is too long ({words} words): {rendered}"
        );
    }
}

/// Neither tool offered renders nothing — the guard behind the gate, so a
/// future caller that registers the section unconditionally still cannot name
/// a tool the session lacks.
#[test]
fn memory_write_instruction_is_empty_without_a_write_tool() {
    assert!(memory_write_instruction(false, false, false).is_empty());
    let rendered = MemoryWriteSection::new(false, false, false)
        .build(&prompt_context(LearnedContextData::default()))
        .unwrap();
    assert!(rendered.is_empty(), "{rendered}");
}

#[test]
fn write_tool_gate_needs_a_registered_and_visible_tool() {
    let tools = named(&["shell", "memory_store"]);
    let none: Vec<Box<dyn Tool>> = Vec::new();
    // Registered, no filter (empty visible set = wildcard): offered.
    assert!(any_tool_offered(
        &MEMORY_WRITE_TOOLS,
        &tools,
        &none,
        &visible(&[])
    ));
    // Registered and allowed by the filter: offered.
    assert!(any_tool_offered(
        &MEMORY_WRITE_TOOLS,
        &tools,
        &none,
        &visible(&["memory_store"])
    ));
    // Registered but filtered out by the agent's scope: not offered.
    assert!(!any_tool_offered(
        &MEMORY_WRITE_TOOLS,
        &tools,
        &none,
        &visible(&["shell"])
    ));
    // Allowed by the filter but never registered on this agent: not offered.
    assert!(!any_tool_offered(
        &MEMORY_WRITE_TOOLS,
        &named(&["shell"]),
        &none,
        &visible(&["memory_store", "save_preference"])
    ));
}

#[test]
fn write_tool_gate_counts_delegation_tools_and_either_write_tool() {
    let none: Vec<Box<dyn Tool>> = Vec::new();
    // A write tool synthesised on the delegation side counts the same.
    assert!(any_tool_offered(
        &MEMORY_WRITE_TOOLS,
        &none,
        &named(&["save_preference"]),
        &visible(&[])
    ));
    // The read-side list is disjoint: a session with only retrieval tools gets
    // no write rule, and a session with only write tools gets no read rule.
    let readers = named(&["memory_recall"]);
    assert!(any_tool_offered(
        &MEMORY_READ_TOOLS,
        &readers,
        &none,
        &visible(&[])
    ));
    assert!(!any_tool_offered(
        &MEMORY_WRITE_TOOLS,
        &readers,
        &none,
        &visible(&[])
    ));
}

// ── The write rule reaches a delegated agent (#6200) ─────────────────────────

/// An agent whose only write path is the delegate gets the rule, and the rule
/// names the delegate.
///
/// This is the write-side twin of the gap #6183 closed on the read side. The
/// orchestrator is configured exactly this way — its visible set carries
/// `manage_profile_memory` and neither direct tool — so before this it held a
/// write path and no rule about using it: the #6048 case, "got it, saved" with
/// no tool call behind it.
///
/// Naming matters as much as presence. Pointing it at `memory_store`, a tool it
/// cannot see, is the failure `any_tool_offered` exists to prevent.
#[test]
fn a_delegate_only_agent_gets_the_write_rule_naming_the_delegate() {
    let delegate = named(&[MEMORY_WRITE_DELEGATE_TOOL]);
    let none: Vec<Box<dyn Tool>> = Vec::new();

    assert!(
        any_tool_offered(
            &[MEMORY_WRITE_DELEGATE_TOOL],
            &delegate,
            &none,
            &visible(&[])
        ),
        "the gate must see the delegate"
    );

    let rendered = memory_write_instruction(false, false, true);
    assert!(
        rendered.contains(MEMORY_WRITE_DELEGATE_TOOL),
        "the rule must name the delegate: {rendered}"
    );
    // #6200 review (Codex P1). `ArchetypeDelegationTool` defaults an omitted
    // `blocking` to `false` and dispatches async, returning before the worker
    // runs. Without demanding `blocking: true` this section would tell the model
    // to confirm a save that has not happened — #6048 arriving by a new route,
    // through the very rule meant to prevent it.
    assert!(
        rendered.contains("blocking: true"),
        "a delegated write must be demanded as blocking, or the reply can \
         confirm before the write lands: {rendered}"
    );
    for absent in [MEMORY_STORE_TOOL, SAVE_PREFERENCE_TOOL] {
        assert!(
            !rendered.contains(absent),
            "the rule named `{absent}`, which a delegate-only agent cannot see: {rendered}"
        );
    }
}

/// An agent holding a direct write tool renders exactly what it rendered before
/// the delegate arm existed.
///
/// The regression that would matter most here is a silent prompt change for
/// every agent that was already working, so the delegate flag is asserted to be
/// inert whenever either direct tool is present.
#[test]
fn a_direct_write_tool_renders_the_same_text_with_or_without_the_delegate() {
    for (preferences, facts) in [(true, true), (true, false), (false, true)] {
        assert_eq!(
            memory_write_instruction(preferences, facts, false),
            memory_write_instruction(preferences, facts, true),
            "the delegate flag changed the text for ({preferences}, {facts})"
        );
    }
    // And the no-write case is still empty rather than falling into the
    // delegate arm by accident.
    assert!(memory_write_instruction(false, false, false).is_empty());
}

/// Read and write both admit their delegate, and neither list admits the
/// other's.
///
/// #6183 fixed the read side and left the write side behind; the two drifting
/// apart is what produced a half-fixed release. Pinning both directions here
/// makes that specific mistake fail a test rather than ship.
#[test]
fn read_and_write_rules_are_symmetric_about_their_delegates() {
    let none: Vec<Box<dyn Tool>> = Vec::new();
    let readers = named(&["retrieve_memory"]);
    let writers = named(&[MEMORY_WRITE_DELEGATE_TOOL]);

    assert!(
        any_tool_offered(&MEMORY_READ_TOOLS, &readers, &none, &visible(&[])),
        "the read list must admit its delegate"
    );
    assert!(
        !any_tool_offered(&MEMORY_READ_TOOLS, &writers, &none, &visible(&[])),
        "the read list must not admit the write delegate"
    );
    assert!(
        !any_tool_offered(
            &[MEMORY_WRITE_DELEGATE_TOOL],
            &readers,
            &none,
            &visible(&[])
        ),
        "the write delegate must not be satisfied by the read delegate"
    );
}
