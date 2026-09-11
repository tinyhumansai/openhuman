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

use crate::openhuman::agent::context::prompt::{PromptContext, PromptSection};
use crate::openhuman::tools::traits::Tool;
use anyhow::Result;
use std::collections::HashSet;

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

        let mut out = String::from("## Your standing preferences\n\n");
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
/// anything the user mentioned in past sessions — and never to claim something
/// is not stored without having looked (#6040).
///
/// The text is frozen at compile time — no I/O at build time.
/// Register this section after [`LearnedContextSection`] in the prompt-section
/// composition order (see `SystemPromptBuilder::with_defaults`). It is added
/// whenever a retrieval tool is registered and visible, independently of
/// `learning.enabled`: the instruction is about the memory tools, which exist
/// whether or not the learning subsystem runs.
pub struct MemoryAccessSection;

/// The static prose injected into every system prompt. Kept at ≤ 100 words
/// (the composition test pins that ceiling).
pub const MEMORY_ACCESS_INSTRUCTION: &str = "\
## Memory access\n\
\n\
Before answering questions involving named people, projects, threads, prior \
decisions, recurring topics, or anything the user has mentioned in past sessions, \
call `memory_recall` (or `memory_search` for keyword lookups) to retrieve \
relevant context. Questions about the user themselves — favourites, idols, \
people, plans, habits — always warrant a retrieval first. Never say something is \
not stored or not remembered unless a retrieval you just ran returned nothing. \
Surface what matters in your reply; don't stitch together continuity from prompt \
history alone. Skip retrieval for purely procedural requests where prior context \
isn't relevant.";

impl PromptSection for MemoryAccessSection {
    fn name(&self) -> &str {
        "memory_access"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(MEMORY_ACCESS_INSTRUCTION.to_string())
    }
}

// ── MemoryWriteSection ────────────────────────────────────────────────────────

/// Instruction that turns "remember this" into a write before the reply.
///
/// The read-side rule above tells the model when to look. Nothing told it that
/// an explicit request to remember must become a tool call, or that it may not
/// say "saved" without one — so, left to itself, it narrated a save it never
/// made and the next chat had nothing to find (#6048).
///
/// It names only the routes this session actually holds. Naming both
/// unconditionally would point a profile that carries just one of them at a
/// tool it cannot see — the failure [`any_tool_offered`] exists to prevent
/// (review finding).
pub struct MemoryWriteSection {
    preferences: bool,
    facts: bool,
    delegate: bool,
}

impl MemoryWriteSection {
    /// `preferences` = `save_preference` is offered here, `facts` =
    /// `memory_store` is, `delegate` = [`MEMORY_WRITE_DELEGATE_TOOL`] is.
    #[must_use]
    pub fn new(preferences: bool, facts: bool, delegate: bool) -> Self {
        Self {
            preferences,
            facts,
            delegate,
        }
    }
}

/// The instruction for a session offering `save_preference` (`preferences`)
/// and/or `memory_store` (`facts`).
///
/// Kept at ≤ 80 words in every variant (the composition test pins the ceiling).
/// Empty when neither tool is offered — which is also when nothing registers
/// the section, so the empty string is a guard, not a path in normal use.
#[must_use]
pub fn memory_write_instruction(preferences: bool, facts: bool, delegate: bool) -> String {
    let route = match (preferences, facts) {
        (true, true) => "— `save_preference` for preferences, `memory_store` for everything else",
        (true, false) => "with `save_preference`",
        (false, true) => "with `memory_store`",
        // Only when neither direct tool is held, so an agent that has one
        // renders exactly the text it rendered before this arm existed.
        //
        // `blocking: true` is not a stylistic detail, it is what makes the
        // sentence above true (#6200 review). `ArchetypeDelegationTool`
        // defaults an omitted `blocking` to `false` and dispatches
        // `PreferAsync`, which hands back an immediate reference while the
        // worker runs later — so a model told merely to "write with
        // `manage_profile_memory`" would confirm a save that had not happened,
        // which is the #6048 bug arriving by a new route. The argument is
        // advertised on the tool's own schema, so this is a demand the model
        // can actually satisfy.
        (false, false) if delegate => {
            "with `manage_profile_memory` (pass `blocking: true` so the write \
             gates your reply)"
        }
        (false, false) => return String::new(),
    };
    format!(
        "## Remembering\n\n\
         When the user asks you to remember, note, or keep something — a date, \
         plan, person, decision, or preference — write it before you confirm \
         {route}. Never say saved, noted, or remembered unless that write \
         succeeded in this turn; if it failed or was refused, say so instead."
    )
}

impl PromptSection for MemoryWriteSection {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(memory_write_instruction(
            self.preferences,
            self.facts,
            self.delegate,
        ))
    }
}

/// The retrieval tools [`MemoryAccessSection`] is keyed on.
///
/// `retrieve_memory` is the **delegate** to the memory sub-agent, synthesised
/// from `delegate_name` in `memory/agent/agent/agent.toml`, and it belongs here
/// for the same reason the two direct tools do: the section is a rule about
/// what the model must do before claiming something is not stored, and an agent
/// holding the delegate can retrieve. Keying only on the direct names meant an
/// orchestrator whose memory arrives by delegation — which is how the
/// orchestrator is configured — got the tool and no rule about using it. That
/// is the shape of the bug #6040 and #6048 were both filed for.
///
/// [`any_tool_offered`] already receives the delegation tools separately, so
/// this needed no new plumbing; the list was simply the wrong list.
pub const MEMORY_READ_TOOLS: [&str; 3] = ["memory_recall", "memory_search", "retrieve_memory"];

/// The tool a preference is written through.
pub const SAVE_PREFERENCE_TOOL: &str = "save_preference";

/// The tool every other remembered fact is written through.
pub const MEMORY_STORE_TOOL: &str = "memory_store";

/// The delegate an agent writes through when it holds neither direct write
/// tool.
///
/// Synthesised from `profile_memory_agent`'s `delegate_name`, and the write-side
/// counterpart of `retrieve_memory` in [`MEMORY_READ_TOOLS`]. The orchestrator
/// is configured this way: its visible set carries this delegate and neither
/// [`MEMORY_STORE_TOOL`] nor [`SAVE_PREFERENCE_TOOL`], so keying the section on
/// the direct pair alone dropped the rule for the agent that needed it most —
/// the #6048 case, "got it, saved" with no tool call behind it.
///
/// The section's promise survives the indirection **only under a blocking
/// delegation**. `profile_memory_agent` holds both direct tools, so a delegated
/// write reaches the same store — but `ArchetypeDelegationTool` defaults to an
/// async dispatch that returns before the worker runs, so the route text demands
/// `blocking: true`. Without that the parent could confirm a save that had not
/// happened yet, which is exactly the bug this section exists to prevent.
pub const MEMORY_WRITE_DELEGATE_TOOL: &str = "manage_profile_memory";

/// The writing routes [`MemoryWriteSection`] is keyed on, delegate included.
///
/// Mirrors [`MEMORY_READ_TOOLS`], which lists `retrieve_memory` beside its two
/// direct tools for the same reason. The live gate in `add_memory_prompt_sections`
/// asks about each of these separately rather than reading this array — the
/// section names the route it found, so it cannot treat them interchangeably —
/// but a reader reaching for "what does the write section care about" should get
/// the whole answer here (#6200 review).
pub const MEMORY_WRITE_TOOLS: [&str; 3] = [
    MEMORY_STORE_TOOL,
    SAVE_PREFERENCE_TOOL,
    MEMORY_WRITE_DELEGATE_TOOL,
];

/// Whether any of `names` is registered on this session **and** survives tool
/// filtering.
///
/// Both are required: a tool can be registered but filtered out by the agent's
/// scope, or allowed by the filter but never registered on this agent. An empty
/// `visible` set means "no filter" (the wildcard / orchestrator path), so any
/// registered tool is reachable. A section that tells the model to call a tool
/// it cannot see would only teach it to apologise.
pub fn any_tool_offered(
    names: &[&str],
    tools: &[Box<dyn Tool>],
    delegation_tools: &[Box<dyn Tool>],
    visible: &HashSet<String>,
) -> bool {
    names.iter().any(|name| {
        let registered = tools
            .iter()
            .chain(delegation_tools)
            .any(|tool| tool.name() == *name);
        let allowed_by_filter = visible.is_empty() || visible.contains(*name);
        registered && allowed_by_filter
    })
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
/// Async because the facet store moved behind the memory driver: this used to
/// be a synchronous SQLite read, and is now a driver call. The caller should
/// keep both this path and the existing KV-namespace path active until the KV
/// path is removed in a follow-up phase.
pub async fn load_learned_from_cache(
    cache: &crate::openhuman::agent::learning::cache::FacetCache,
) -> Vec<String> {
    let facets = match cache.list_active().await {
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
    use std::collections::BTreeMap;
    use tinymemory_api::provider::ProfileFacet;
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
            let pinned = if f.user_state == tinymemory_api::provider::UserState::Pinned {
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
#[path = "prompt_sections_tests_2_tests.rs"]
mod tests;
