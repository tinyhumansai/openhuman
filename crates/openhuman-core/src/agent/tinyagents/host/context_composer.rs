//! Host adapter: [`tinyagents_harness::host::ContextComposer`] backed by
//! OpenHuman's prompt pipeline.
//!
//! # Which OpenHuman domains this adapts
//!
//! * [`crate::agent::prompts`] (re-exported as
//!   `crate::agent::prompts`) — `SystemPromptBuilder`,
//!   `PromptContext`, `ConnectedIntegration`,
//!   `ToolCallFormat`, `load_agents_md_layers`, `render_connected_identities`.
//!   The `SOUL.md` / `IDENTITY.md` / `ROLE.md` bootstrap files under
//!   `crates/openhuman-core/src/agent/prompts/` are loaded (and synced to the workspace)
//!   by `IdentitySection` inside `SystemPromptBuilder::build`, so this adapter
//!   never reads them itself.
//! * [`crate::config::Config`] — supplies `workspace_dir`,
//!   `action_dir`, the default model, and the `agents_md_enabled` gate.
//! * [`crate::security::credentials::identity::peek_credential_user_identity`] — the
//!   non-secret `id`/`name`/`email` triple. Deliberately read through the
//!   *identity peek*, which is the accessor that strips credential material;
//!   this adapter must not reach for a richer user record to fill the prompt.
//!
//! Prompt assembly is **not** reimplemented here. Everything this file does is
//! translate a [`TurnContextRequest`] into a `PromptContext` and hand it to the
//! existing `SystemPromptBuilder::with_defaults()` chain — the same chain
//! `agent::session_host::turn::context::build_system_prompt` uses. That
//! keeps one source of truth for section ordering, the grounding contract, and
//! the global style suffix.
//!
//! # Contract mismatches resolved here
//!
//! 1. **Per-turn call vs. frozen prefix.** The crate consults a composer on
//!    *every* turn (`context_composer.rs`: "a host whose learned context …
//!    changes mid-session needs the later turns to see it"). OpenHuman's
//!    pipeline goes the other way: `SystemPromptBuilder::build`'s rustdoc
//!    states the rendered bytes are "intended to be **frozen for the whole
//!    session**" so the inference backend's prefix cache hits. Both are
//!    honoured by construction — every input this adapter feeds the builder is
//!    a snapshot captured at *construction* time (config, connected
//!    integrations), so recomposing per turn yields byte-identical
//!    output unless the host deliberately builds a new composer. The two
//!    genuinely time-varying sections in the default chain
//!    (`DateTimeSection`, and the on-disk files `IdentitySection` reads) are
//!    OpenHuman's pre-existing behaviour on the main-agent path, not something
//!    this seam introduces.
//! 2. **`thread_id` and `user_text` are unused inputs.** `PromptContext` has no
//!    thread field and no turn-text field; this adapter only logs the thread
//!    id for correlation.
//! 3. **`preamble` returns an empty `Vec`, always.** OpenHuman has no separate
//!    preamble concept here: everything the system prompt needs is rendered
//!    *into* it as `PromptSection`s. Per the trait's
//!    "Empty is not failure" note this is the normal, correct answer, not a
//!    gap — synthesising extra messages here would duplicate content the
//!    system prompt already carries.
//!
//! # Policy
//!
//! The `agents_md_enabled` config gate is honoured: when it is off, both
//! AGENTS.md layers are handed to the builder as `None` rather than being
//! loaded. `load_agents_md_layers` performs the `O_NOFOLLOW` path hardening
//! that keeps a symlinked project-layer `AGENTS.md` from leaking arbitrary
//! host files into the prompt (and thence to the inference provider); this
//! adapter goes through it rather than reading the files itself.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::error::{Result as TinyAgentsResult, TinyAgentsError};
use tinyagents_harness::host::{ContextComposer, TurnContextRequest};
use tinyinference_llm::message::Message;

use crate::agent::prompts::{
    load_agents_md_layers, render_connected_identities, AgentsMdContent, ConnectedIntegration,
    PromptContext, PromptTool, SystemPromptBuilder, ToolCallFormat,
};
use crate::config::{Config, DEFAULT_MODEL};
use crate::skills::Workflow;

/// Composes OpenHuman's system prompt for a TinyAgents turn.
///
/// Holds a snapshot of everything the prompt pipeline needs that is *not*
/// carried on [`TurnContextRequest`]. Snapshot rather than live handles is
/// deliberate: it is what makes repeated `compose_system_prompt` calls
/// byte-stable, which is the KV-cache contract described in the module doc.
///
/// The struct is cheap to clone-construct and holds no locks, so a caller that
/// genuinely needs mid-session refresh (new integration connected) rebuilds the composer rather than
/// mutating it.
pub struct OpenHumanContextComposer {
    /// Host config — source of the two path roots, the fallback model name,
    /// and the `agents_md_enabled` gate.
    config: Arc<Config>,
    /// Model name rendered into the prompt's runtime section. Defaults to
    /// `config.default_model` (then [`DEFAULT_MODEL`]) because the crate's
    /// `TurnContextRequest` carries no model — model resolution is a separate
    /// host capability (`ModelResolver`) and this seam must not second-guess
    /// it.
    model_name: String,
    /// How the tool catalogue renders. Left at the OpenHuman default
    /// ([`ToolCallFormat::PFormat`]) unless the caller pins it, since the
    /// authoritative value lives on the agent's tool dispatcher, which is not
    /// reachable from a `TurnContextRequest`.
    tool_call_format: ToolCallFormat,
    /// Connected Composio integrations, pre-fetched. Empty is a valid state
    /// (nothing connected, or the caller has not fetched yet) and renders as
    /// an absent section rather than an error.
    connected_integrations: Vec<ConnectedIntegration>,
    /// The section chain. Built once so per-turn composition is just a render.
    builder: SystemPromptBuilder,
}

impl OpenHumanContextComposer {
    /// A composer over `config` with no integrations.
    ///
    /// This is the honest zero state, not a degraded one: a fresh install with
    /// nothing connected composes exactly this prompt.
    pub fn new(config: Arc<Config>) -> Self {
        let model_name = config
            .default_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        Self {
            config,
            model_name,
            tool_call_format: ToolCallFormat::default(),
            connected_integrations: Vec::new(),
            builder: SystemPromptBuilder::with_defaults(),
        }
    }

    /// Pins the model name rendered into the runtime section.
    pub fn with_model_name(mut self, model_name: impl Into<String>) -> Self {
        self.model_name = model_name.into();
        self
    }

    /// Loads the two AGENTS.md layers, honouring the `agents_md_enabled` gate.
    ///
    /// Split out so the gate has exactly one enforcement point and the tests
    /// can pin the disabled branch without rendering a whole prompt.
    fn agents_md(&self) -> AgentsMdContent {
        if self.config.agent.agents_md_enabled {
            load_agents_md_layers(&self.config.workspace_dir, &self.config.action_dir)
        } else {
            tracing::debug!(
                target: "tinyagents",
                "[tinyagents][context_composer] agents_md_enabled is off; skipping AGENTS.md injection"
            );
            AgentsMdContent::default()
        }
    }
}

#[async_trait]
impl ContextComposer for OpenHumanContextComposer {
    /// Renders the full OpenHuman system prompt for this turn.
    ///
    /// Returns `Err(TinyAgentsError::Validation)` only when a
    /// [`crate::agent::prompts::PromptSection`] itself fails — a
    /// genuine fault, per the trait's "reserve it for genuine faults" rule.
    /// An empty section is skipped by the builder, never surfaced as an error.
    async fn compose_system_prompt(&self, req: &TurnContextRequest) -> TinyAgentsResult<String> {
        tracing::debug!(
            target: "tinyagents",
            agent_id = %req.agent_id,
            thread_id = %req.thread_id.as_str(),
            has_user_text = req.has_user_text(),
            "[tinyagents][context_composer] composing system prompt"
        );

        // Every borrowed field of `PromptContext` needs an owner that outlives
        // the context, so the empties are bound here rather than inline.
        //
        // `tools` and `visible_tool_names` are empty on purpose: the crate's
        // `TurnContextRequest` carries no tool set, and the tool catalogue is
        // owned by the runtime's own tool registry rather than by this seam.
        // An empty `visible_tool_names` is also the *non-orchestrator* signal
        // that `IdentitySection` keys off, which is the correct default for a
        // composer that does not know it is driving a delegator.
        //
        // TODO(phase4): once the runtime exposes its resolved tool set to the
        // host (a `ToolCatalog`-shaped capability would be the natural seam),
        // feed it through `PromptTool::from_tools` / `PromptTool::with_schema`
        // so `ToolsSection` renders a real catalogue instead of nothing.
        let prompt_tools: Vec<PromptTool<'_>> = Vec::new();
        let visible_tool_names: HashSet<String> = HashSet::new();
        // TODO(phase4): installed workflows live in
        // `crate::skill_registry` / `skills`, behind the `skills`
        // compile-time gate. Wiring them needs a gated fetch plus a decision
        // about the disabled build, so they are left empty here rather than
        // guessed at.
        let workflows: Vec<Workflow> = Vec::new();
        let agents_md = self.agents_md();

        let ctx = PromptContext {
            workspace_dir: &self.config.workspace_dir,
            model_name: &self.model_name,
            agent_id: &req.agent_id,
            tools: &prompt_tools,
            workflows: &workflows,
            // The dispatcher's tool-protocol preamble belongs to the runtime's
            // dispatcher, which this seam cannot see. Empty renders nothing.
            dispatcher_instructions: "",
            visible_tool_names: &visible_tool_names,
            tool_call_format: self.tool_call_format,
            connected_integrations: &self.connected_integrations,
            connected_identities_md: render_connected_identities(),
            user_identity: crate::security::credentials::identity::peek_credential_user_identity(),
            // TODO(phase4): the master agent's personality roster is built
            // from the profiles domain (`crate::profiles`); the
            // existing main-agent path leaves this empty too (see the
            // `personality_roster: vec![]` TODO in
            // `agent/session_host/turn/context.rs`), so this matches
            // current behaviour rather than regressing it.
            personality_roster: Vec::new(),
            agents_md_global: agents_md.global,
            agents_md_local: agents_md.local,
        };

        self.builder.build(&ctx).map_err(|e| {
            TinyAgentsError::Validation(format!("system prompt composition failed: {e:#}"))
        })
    }

    /// Always empty — see mismatch (3) in the module doc.
    ///
    /// OpenHuman renders everything as prompt *sections*, so there is nothing left over to prepend as messages.
    /// Returning `Ok(vec![])` is the trait's documented normal case.
    async fn preamble(&self, _req: &TurnContextRequest) -> TinyAgentsResult<Vec<Message>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
#[path = "context_composer_tests.rs"]
mod tests;
