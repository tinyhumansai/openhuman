//! Agent tool: read a compiled persona flavour profile (issue #5172).
//!
//! Persona ingestion (driver-side) distills a person's coding-agent history
//! into seven [`PersonaFacet`] flavoured trees (communication, coding style,
//! stack, workflow, environment, directives, anti-preferences), each compiled
//! into a small prompt-ready markdown profile. Until this tool, nothing
//! surfaced those compiled profiles to the agent loop — the ingested data sat
//! unread. `memory_flavour` lets an agent pull one facet's profile on demand.
//!
//! Strictly read-only: it never ingests, seals, or otherwise creates persona
//! evidence. The only disk write it can trigger is the driver re-staging the
//! fixed-path compiled artifact — a pure, idempotent projection of the tree's
//! existing root node, not new memory content.
//!
//! # This file is why `FlavourProfile` exists (#5560)
//!
//! It reached `tinycortex::memory::tree::{store::get_tree_by_scope,
//! compile_flavoured_root, flavoured_root_abs_path}` directly, and all three
//! take a `tinycortex::memory::MemoryConfig` — so the file was pinned not by a
//! missing capability but by the fact that nothing host-side could build that
//! config without reproducing the engine's own mapping. `MemoryTree::
//! flavour_profile` collapses the entire lookup behind one scope-shaped
//! question, and the config is built on the driver's side of the bus where it
//! belongs. What stays here is the vocabulary ([`PersonaFacet`] and its three
//! string mappings) and the presentation ([`body_after_front_matter`]).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// The seven persona facets, host-side (#5560).
///
/// This was `tinycortex::memory::persona::PersonaFacet`, and it came home
/// because it is a pure value type: a field-less enum whose whole behaviour is
/// three total string mappings. Nothing about it needs the engine — the engine
/// functions this file calls take the resulting `String`/`&str`, never the enum
/// — so a host copy is the same value under a different path, not a
/// translation.
///
/// # The strings are an on-disk contract, not cosmetics
///
/// [`Self::tree_scope`] is the **key a flavoured tree is stored under**.
/// Persona ingestion writes `persona/<facet>` into `mem_tree_trees`, and
/// `get_tree_by_scope` finds it by exact string match. So the mappings below
/// are reproduced verbatim from the engine, and a "tidy-up" that renames one
/// (`coding_style` → `codingStyle`, say) does not fail a build or throw — it
/// silently stops finding a tree that is still there, and `memory_flavour`
/// starts answering "No profile built yet" forever.
///
/// [`Self::parse_loose`]'s alias table is the agent-facing half of the same
/// contract: an LLM emits `tone` or `pet_peeves`, and dropping an alias
/// narrows what the tool accepts. [`Self::heading`] is display-only and the one
/// mapping here that is safe to reword.
///
/// The engine's enum carries three more members this host never reads — `ALL`
/// (the pack's fixed compile order), `default_ask` (per-facet ingestion
/// prompts) and its serde derives. They are ingestion concerns and are
/// deliberately not copied: an unused copy is a second thing to keep in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersonaFacet {
    /// Tone, verbosity, directness, phrasing quirks, how they give feedback.
    Communication,
    /// Naming, structure, comments, error handling, testing habits.
    CodingStyle,
    /// Languages, frameworks, libraries, recurring architectural choices.
    Stack,
    /// Branching/commit granularity, plan-first vs. dive-in, PR habits.
    Workflow,
    /// Editors/harnesses, CLIs, package managers, OS.
    Environment,
    /// Explicit standing rules (mostly T0, near-verbatim).
    Directives,
    /// Pet peeves: things they correct agents for, revert, or forbid.
    AntiPreferences,
}

impl PersonaFacet {
    /// Stable string form. Verbatim from the engine — see the type's docs for
    /// why this one is not free to change.
    fn as_str(self) -> &'static str {
        match self {
            PersonaFacet::Communication => "communication",
            PersonaFacet::CodingStyle => "coding_style",
            PersonaFacet::Stack => "stack",
            PersonaFacet::Workflow => "workflow",
            PersonaFacet::Environment => "environment",
            PersonaFacet::Directives => "directives",
            PersonaFacet::AntiPreferences => "anti_preferences",
        }
    }

    /// Human-facing section heading used in error and "not built" messages.
    /// Display-only, so this is the one mapping here that may be reworded.
    pub(crate) fn heading(self) -> &'static str {
        match self {
            PersonaFacet::Communication => "Communication style",
            PersonaFacet::CodingStyle => "Coding style",
            PersonaFacet::Stack => "Stack",
            PersonaFacet::Workflow => "Workflow",
            PersonaFacet::Environment => "Environment",
            PersonaFacet::Directives => "Directives",
            PersonaFacet::AntiPreferences => "Anti-preferences",
        }
    }

    /// Flavoured-tree scope for this facet (`persona/<facet>`) — the exact key
    /// the tree is persisted under.
    pub(crate) fn tree_scope(self) -> String {
        format!("persona/{}", self.as_str())
    }

    /// Parse the loose forms an LLM might emit.
    pub(crate) fn parse_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
            "communication" | "comms" | "tone" => Some(PersonaFacet::Communication),
            "coding_style" | "code_style" | "coding" | "style" => Some(PersonaFacet::CodingStyle),
            "stack" | "tech_stack" | "technology" => Some(PersonaFacet::Stack),
            "workflow" | "process" => Some(PersonaFacet::Workflow),
            "environment" | "env" | "tooling" => Some(PersonaFacet::Environment),
            "directives" | "rules" | "directive" => Some(PersonaFacet::Directives),
            "anti_preferences" | "anti_preference" | "antipreferences" | "dislikes"
            | "pet_peeves" => Some(PersonaFacet::AntiPreferences),
            _ => None,
        }
    }
}

/// The seven valid `flavour` slugs, for error messages.
const VALID_FLAVOURS: &str =
    "communication, coding_style, stack, workflow, environment, directives, anti_preferences";

/// Let the agent read the compiled persona profile for one facet.
pub struct MemoryFlavourTool {
    config: Arc<Config>,
}

impl MemoryFlavourTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

/// Strip the YAML front matter the flavoured-root compile writes
/// (`---\n...\n---\n<body>`) and return just the body. Front-matter field
/// values are single-line (the compiler's `yaml_quote` collapses interior
/// newlines), so the first `\n---\n` after the opening delimiter is always the
/// closing one.
///
/// This is presentation, and presentation is the caller's:
/// [`MemoryTree::flavour_profile`](crate::openhuman::memory::api::provider::MemoryTree::flavour_profile)
/// answers with the **full** artifact because the front matter is part of what
/// was compiled, and only this side knows it wants prose.
fn body_after_front_matter(content: &str) -> &str {
    match content.strip_prefix("---\n") {
        Some(rest) => match rest.find("\n---\n") {
            Some(pos) => &rest[pos + "\n---\n".len()..],
            // Malformed front matter (opener present but no closer): fall
            // back to everything after the opening delimiter rather than
            // the raw content, so the opener itself is never leaked.
            None => rest,
        },
        None => content,
    }
}

/// Outcome of [`lookup_flavour`] — split from a hard `Err` so a caller can
/// distinguish "bad input" (never reached the store) from "reached the store
/// and here's what it found (or didn't, or a lookup itself failed)".
pub(crate) enum FlavourLookup {
    /// A compiled profile body, ready to hand to the agent/node.
    Profile(String),
    /// No profile has been built yet for this facet — not an error, just
    /// empty (persona ingestion hasn't run, or produced nothing for this
    /// facet yet).
    NotBuilt(String),
    /// The tree lookup or compile step itself failed (I/O, corrupt tree,
    /// …) — distinct from `NotBuilt` because this IS an error, just one
    /// discovered after `flavour_raw` was already validated.
    Failed(String),
}

/// The lookup shared by [`MemoryFlavourTool::execute`] and the tinyflows
/// `memory` node's `flavour` operation
/// (`OpenHumanMemory::flavour` in `crate::openhuman::flows::tinyflows::memory_adapter`)
/// — both surfaces read the exact same flavoured-tree path, so there is only
/// one place that knows how a `flavour` slug resolves to a compiled profile.
///
/// `async` since #5560: the read crosses the module bus rather than running
/// in-process. Both call sites were already `async fn`s, so nothing is bridged.
///
/// `Err` is reserved for input the caller should have caught before ever
/// reaching the store (empty/unknown `flavour_raw`); everything the store
/// itself can report — hit, miss, or lookup failure — comes back as `Ok` of
/// the matching [`FlavourLookup`] variant so callers can shape each case
/// (tool result vs. node output) however their surface needs.
pub(crate) async fn lookup_flavour(
    config: &Config,
    flavour_raw: &str,
) -> Result<FlavourLookup, String> {
    let flavour_raw = flavour_raw.trim();
    if flavour_raw.is_empty() {
        return Err("'flavour' cannot be empty".to_string());
    }

    let facet = PersonaFacet::parse_loose(flavour_raw).ok_or_else(|| {
        format!("Unknown flavour '{flavour_raw}'. Valid flavours: {VALID_FLAVOURS}")
    })?;

    let scope = facet.tree_scope();
    let heading = facet.heading();

    tracing::debug!(
        target: "memory_flavour",
        flavour = flavour_raw,
        facet = ?facet,
        "[memory_flavour] entry"
    );

    // The whole lookup this function used to run in-process — build a
    // `MemoryConfig`, try the compiled artifact on disk, fall back to
    // `get_tree_by_scope` + `compile_flavoured_root` — is one contract member
    // now (#5560). That is why the door exists: the three TinyCortex calls all
    // took a `tinycortex::memory::MemoryConfig`, and building one host-side
    // meant reproducing the engine's `Config` → `MemoryConfig` mapping field by
    // field, including an `embedding.provider` this path did not read but the
    // next edit to it might have.
    //
    // The driver runs the same two steps in the same order and applies the same
    // built/not-built rule (a tree whose compiled root has an empty body is
    // "not built", never an empty profile), so the three outcomes below are the
    // three this function always had.
    let guard = crate::openhuman::memory::binding::for_config(config)?.guard();
    let Some(tree) = guard.as_tree() else {
        tracing::warn!(
            target: "memory_flavour",
            driver = %guard.driver_id(),
            "[memory_flavour] driver does not serve Tree"
        );
        return Ok(FlavourLookup::Failed(format!(
            "Failed to look up the {heading} profile: driver '{}' does not serve Tree",
            guard.driver_id()
        )));
    };

    match tree.flavour_profile(&scope).await {
        // The member answers the **full compiled artifact, front matter
        // included** — presentation is deliberately the caller's — so the strip
        // stays here, exactly as it was.
        Ok(Some(markdown)) => {
            let body = body_after_front_matter(&markdown);
            if body.trim().is_empty() {
                // Unreachable against a conforming driver, which folds this
                // into `Ok(None)`. Kept because the alternative is handing a
                // model an empty string that reads as "this person has no
                // communication style".
                Ok(FlavourLookup::NotBuilt(format!(
                    "No profile built yet for {heading}. Run persona ingestion first, then try \
                     again."
                )))
            } else {
                tracing::debug!(
                    target: "memory_flavour",
                    flavour = flavour_raw,
                    body_len = body.len(),
                    "[memory_flavour] compiled profile returned"
                );
                Ok(FlavourLookup::Profile(body.to_string()))
            }
        }
        Ok(None) => {
            tracing::debug!(
                target: "memory_flavour",
                flavour = flavour_raw,
                "[memory_flavour] no flavoured tree exists yet"
            );
            Ok(FlavourLookup::NotBuilt(format!(
                "No profile built yet for {heading}. Run persona ingestion first, then try \
                 again."
            )))
        }
        Err(err) => {
            tracing::warn!(
                %err,
                flavour = flavour_raw,
                "[memory_flavour] failed to look up flavoured tree"
            );
            Ok(FlavourLookup::Failed(format!(
                "Failed to look up the {heading} profile: {err}"
            )))
        }
    }
}

#[async_trait]
impl Tool for MemoryFlavourTool {
    fn name(&self) -> &str {
        "memory_flavour"
    }

    fn description(&self) -> &str {
        "Read the compiled persona profile for one distillation facet, built from this \
         person's coding-agent history. Valid `flavour` values: communication (tone, \
         verbosity, feedback style), coding_style (naming, structure, testing habits), \
         stack (languages, frameworks, architecture), workflow (branching, PR habits, \
         parallelism), environment (editors, harnesses, CLIs, OS), directives (explicit \
         standing rules), anti_preferences (things to never do). Returns markdown prose, or \
         a clear message if no profile has been built yet. Read-only."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "flavour": {
                    "type": "string",
                    "description": "Which persona facet to read: communication, coding_style, \
                        stack, workflow, environment, directives, or anti_preferences."
                }
            },
            "required": ["flavour"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Genuinely read-only: this tool never ingests, seals, or writes
        // memory content. Overridden explicitly (not relying on the trait
        // default) so a future default change can't silently loosen this.
        PermissionLevel::ReadOnly
    }

    fn permission_level_with_args(&self, _args: &serde_json::Value) -> PermissionLevel {
        // No arg combination for this tool escalates past ReadOnly.
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let flavour_raw = args
            .get("flavour")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'flavour' parameter"))?;

        match lookup_flavour(&self.config, flavour_raw).await {
            Err(hard) => Err(anyhow::anyhow!(hard)),
            Ok(FlavourLookup::Profile(body)) => Ok(ToolResult::success(body)),
            Ok(FlavourLookup::NotBuilt(msg)) => Ok(ToolResult::success(msg)),
            Ok(FlavourLookup::Failed(msg)) => Ok(ToolResult::error(msg)),
        }
    }
}

#[cfg(test)]
#[path = "flavour_tests.rs"]
mod tests;
