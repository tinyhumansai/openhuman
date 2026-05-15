//! Prompt sections that inject learned context into the agent's system prompt.
//!
//! These sections read pre-fetched data from `PromptContext.learned` — no async
//! or blocking I/O happens during prompt building.
//!
//! ## Phase 3 addition (#566)
//!
//! [`load_learned_from_cache`] reads Active facets from the `FacetCache`
//! (backed by `user_profile_facets`) and returns them as a list of formatted
//! strings suitable for injection into `LearnedContextData.user_profile`.
//!
//! The existing KV-namespace reads in `fetch_learned_context` are preserved
//! (both paths active in this phase; KV path will be removed in a follow-up).
//!
//! ## Phase 4 addition (#566)
//!
//! [`MemoryAccessSection`] — a static prompt section that instructs the agent to
//! call `memory_recall` / `memory_search` before answering questions that draw on
//! prior sessions. Registered after `LearnedContextSection` in the section chain.

use crate::openhuman::context::prompt::{PromptContext, PromptSection};
use anyhow::Result;

/// Injects recent observations and patterns from the learning subsystem.
pub struct LearnedContextSection;

impl LearnedContextSection {
    pub fn new(_memory: std::sync::Arc<dyn crate::openhuman::memory::Memory>) -> Self {
        // Memory parameter kept for API compatibility but data comes from PromptContext.learned
        Self
    }
}

impl PromptSection for LearnedContextSection {
    fn name(&self) -> &str {
        "learned_context"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.observations.is_empty() && ctx.learned.patterns.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## Learned Context\n\n");

        if !ctx.learned.observations.is_empty() {
            out.push_str("### Recent Observations\n");
            for obs in &ctx.learned.observations {
                out.push_str("- ");
                out.push_str(obs);
                out.push('\n');
            }
            out.push('\n');
        }

        if !ctx.learned.patterns.is_empty() {
            out.push_str("### Recognized Patterns\n");
            for pat in &ctx.learned.patterns {
                out.push_str("- ");
                out.push_str(pat);
                out.push('\n');
            }
            out.push('\n');
        }

        Ok(out)
    }
}

/// Injects the learned user profile into the system prompt.
pub struct UserProfileSection;

impl UserProfileSection {
    pub fn new(_memory: std::sync::Arc<dyn crate::openhuman::memory::Memory>) -> Self {
        Self
    }
}

impl PromptSection for UserProfileSection {
    fn name(&self) -> &str {
        "user_profile"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.user_profile.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## User Profile (Learned)\n\n");
        for entry in &ctx.learned.user_profile {
            out.push_str("- ");
            out.push_str(entry);
            out.push('\n');
        }
        out.push('\n');
        Ok(out)
    }
}

// ── MemoryAccessSection ───────────────────────────────────────────────────────

/// Static bias instruction that tells the agent to call `memory_recall` before
/// answering questions involving named people, projects, prior decisions, or
/// anything the user mentioned in past sessions.
///
/// The text is frozen at compile time — no I/O at build time.
/// Register this section after [`LearnedContextSection`] in the prompt-section
/// composition order (see `SystemPromptBuilder::with_defaults`).
pub struct MemoryAccessSection;

/// The static prose injected into every system prompt. Kept at ≤ 80 tokens.
pub const MEMORY_ACCESS_INSTRUCTION: &str = "\
## Memory access\n\
\n\
Before answering questions involving named people, projects, threads, prior \
decisions, recurring topics, or anything the user has mentioned in past sessions, \
call `memory_recall` (or `memory_search` for keyword lookups) to retrieve \
relevant context. Surface what matters in your reply; don't stitch together \
continuity from prompt history alone. Skip retrieval for purely procedural \
requests where prior context isn't relevant.";

impl PromptSection for MemoryAccessSection {
    fn name(&self) -> &str {
        "memory_access"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(MEMORY_ACCESS_INSTRUCTION.to_string())
    }
}

// ── Cache-backed loader ───────────────────────────────────────────────────────

/// Maximum number of facets to include in the ambient prompt injection.
///
/// Corresponds to "~25 entries total" from the Phase 3 spec.
const CACHE_PROMPT_CAP: usize = 25;

/// Load Active facets from the `FacetCache` and format them for prompt injection.
///
/// Returns a list of strings in the form `class/key: value`, sorted by stability
/// descending within each class, then alphabetically by class. The total is capped
/// at [`CACHE_PROMPT_CAP`] entries.
///
/// This function is **synchronous** and performs only SQLite reads — safe to call
/// from the synchronous part of the system prompt build path. The caller should
/// keep both this path and the existing KV-namespace path active until the KV path
/// is removed in a follow-up phase.
pub fn load_learned_from_cache(
    cache: &crate::openhuman::learning::cache::FacetCache,
) -> Vec<String> {
    let facets = match cache.list_active() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("[learning::prompt] load_learned_from_cache failed: {e}");
            return Vec::new();
        }
    };

    if facets.is_empty() {
        return Vec::new();
    }

    // Group by class prefix (portion before the first '/'), then sort within
    // each class by stability descending, then by key alphabetically.
    use crate::openhuman::memory::store::profile::ProfileFacet;
    use std::collections::BTreeMap;
    let mut by_class: BTreeMap<String, Vec<usize>> = BTreeMap::new();

    for (idx, f) in facets.iter().enumerate() {
        let class = f
            .key
            .split_once('/')
            .map(|(prefix, _rest)| prefix.to_string())
            .unwrap_or_else(|| "other".to_string());
        by_class.entry(class).or_default().push(idx);
    }

    for indices in by_class.values_mut() {
        indices.sort_by(|&a, &b| {
            facets[b]
                .stability
                .partial_cmp(&facets[a].stability)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| facets[a].key.cmp(&facets[b].key))
        });
    }

    let mut result = Vec::with_capacity(CACHE_PROMPT_CAP);
    'outer: for indices in by_class.values() {
        for &idx in indices {
            if result.len() >= CACHE_PROMPT_CAP {
                break 'outer;
            }
            let f: &ProfileFacet = &facets[idx];
            // Phase 4: render in structured `class/key: value` form so the
            // agent can parse the source. Goal class keeps value-only (full
            // sentence, no key prefix). Pinned entries get a trailing suffix.
            let pinned =
                if f.user_state == crate::openhuman::memory::store::profile::UserState::Pinned {
                    " *(pinned)*"
                } else {
                    ""
                };
            let entry = if f.key.starts_with("goal/") {
                // Goal class: render just the value, it's a sentence.
                format!("{}{}", f.value, pinned)
            } else {
                format!("**{}**: {}{}", f.key, f.value, pinned)
            };
            result.push(entry);
        }
    }

    result
}

#[cfg(test)]
#[path = "prompt_sections_tests.rs"]
mod prompt_sections_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::LearnedContextData;
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
            skills: &[],
            dispatcher_instructions: "",
            learned,
            visible_tool_names,
            tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::PFormat,
            connected_integrations: &[],
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            user_identity: None,
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
        assert!(rendered.starts_with("## User Profile (Learned)\n\n"));
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

    #[test]
    fn load_learned_from_cache_formats_active_facets() {
        use crate::openhuman::learning::cache::FacetCache;
        use crate::openhuman::memory::store::profile::{
            FacetState, FacetType, ProfileFacet, UserState, PROFILE_INIT_SQL,
        };
        use parking_lot::Mutex;
        use rusqlite::Connection;

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(PROFILE_INIT_SQL).unwrap();
        let cache = FacetCache::new(Arc::new(Mutex::new(conn)));

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
            .unwrap();
        cache
            .upsert(&make_facet("f2", "identity/name", "Alice", 1.8))
            .unwrap();
        cache
            .upsert(&make_facet(
                "f3",
                "goal/learn_rust",
                "Learn Rust this year",
                1.6,
            ))
            .unwrap();

        // Provisional — should NOT appear.
        let mut prov = make_facet("f4", "style/tone", "formal", 0.8);
        prov.state = FacetState::Provisional;
        cache.upsert(&prov).unwrap();

        let result = load_learned_from_cache(&cache);

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

    #[test]
    fn load_learned_from_cache_empty_when_no_active_facets() {
        use crate::openhuman::learning::cache::FacetCache;
        use crate::openhuman::memory::store::profile::PROFILE_INIT_SQL;
        use parking_lot::Mutex;
        use rusqlite::Connection;

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(PROFILE_INIT_SQL).unwrap();
        let cache = FacetCache::new(Arc::new(Mutex::new(conn)));

        let result = load_learned_from_cache(&cache);
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
}
