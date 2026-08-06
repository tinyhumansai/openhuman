//! Host adapter: [`tinyagents::harness::host::ContextComposer`] backed by
//! OpenHuman's prompt pipeline.
//!
//! # Which OpenHuman domains this adapts
//!
//! * [`crate::openhuman::agent::prompts`] (re-exported as
//!   `crate::openhuman::agent::context::prompt`) — `SystemPromptBuilder`,
//!   `PromptContext`, `LearnedContextData`, `ConnectedIntegration`,
//!   `ToolCallFormat`, `load_agents_md_layers`, `render_connected_identities`.
//!   The `SOUL.md` / `IDENTITY.md` / `HEARTBEAT.md` bootstrap files under
//!   `src/openhuman/agent/prompts/` are loaded (and synced to the workspace)
//!   by `IdentitySection` inside `SystemPromptBuilder::build`, so this adapter
//!   never reads them itself.
//! * [`crate::openhuman::config::Config`] — supplies `workspace_dir`,
//!   `action_dir`, the default model, and the `agents_md_enabled` gate.
//! * [`crate::openhuman::desktop::app_state::peek_cached_current_user_identity`] — the
//!   non-secret `id`/`name`/`email` triple. Deliberately read through the
//!   *cache peek*, which is the accessor that strips credential material; this
//!   adapter must not reach for a richer user record to fill the prompt.
//!
//! Prompt assembly is **not** reimplemented here. Everything this file does is
//! translate a [`TurnContextRequest`] into a `PromptContext` and hand it to the
//! existing `SystemPromptBuilder::with_defaults()` chain — the same chain
//! `agent::harness::session::turn::context::build_system_prompt` uses. That
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
//!    a snapshot captured at *construction* time (config, learned context,
//!    connected integrations), so recomposing per turn yields byte-identical
//!    output unless the host deliberately builds a new composer. The two
//!    genuinely time-varying sections in the default chain
//!    (`DateTimeSection`, and the on-disk files `IdentitySection` reads) are
//!    OpenHuman's pre-existing behaviour on the main-agent path, not something
//!    this seam introduces.
//! 2. **`thread_id` and `user_text` are unused inputs.** `PromptContext` has no
//!    thread field and no turn-text field: OpenHuman scopes learned context
//!    *outside* the prompt layer and pre-fetches it (see every existing
//!    `PromptContext { learned: … }` call site). Rather than invent a
//!    thread-keyed fetch, this adapter takes a caller-supplied
//!    [`LearnedContextData`] snapshot — exactly the established pattern — and
//!    logs the thread id for correlation. See the TODO on
//!    [`OpenHumanContextComposer::learned`].
//! 3. **`preamble` returns an empty `Vec`, always.** OpenHuman has no separate
//!    preamble concept: goals, pinned context, and memory blocks are all
//!    rendered *into* the system prompt as `PromptSection`s. Per the trait's
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
use tinyagents::error::{Result as TinyAgentsResult, TinyAgentsError};
use tinyagents::harness::host::{ContextComposer, TurnContextRequest};
use tinyagents::harness::message::Message;

use crate::openhuman::agent::prompts::{
    load_agents_md_layers, render_connected_identities, AgentsMdContent, ConnectedIntegration,
    LearnedContextData, PromptContext, PromptTool, SystemPromptBuilder, ToolCallFormat,
};
use crate::openhuman::config::{Config, DEFAULT_MODEL};
use crate::openhuman::skills::Workflow;

/// Composes OpenHuman's system prompt for a TinyAgents turn.
///
/// Holds a snapshot of everything the prompt pipeline needs that is *not*
/// carried on [`TurnContextRequest`]. Snapshot rather than live handles is
/// deliberate: it is what makes repeated `compose_system_prompt` calls
/// byte-stable, which is the KV-cache contract described in the module doc.
///
/// The struct is cheap to clone-construct and holds no locks, so a caller that
/// genuinely needs mid-session refresh (new integration connected, learning
/// subsystem produced new reflections) rebuilds the composer rather than
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
    /// Pre-fetched learned context.
    ///
    // TODO(phase4): this should be resolved per `req.thread_id` rather than
    // snapshotted at construction. The real fetch lives in
    // `crate::openhuman::agent::harness::session::turn::context` (see the
    // `LearnedContextData { … }` assembly around `sanitize_learned_entry` /
    // `tree_root_summaries`), which reads the learning store and the memory
    // tree summarizer. It is not a free function and is not thread-keyed
    // today, so exposing it here would mean inventing an API. Callers pass a
    // snapshot via `with_learned_context` in the meantime — the same thing
    // every existing `PromptContext` call site does.
    learned: LearnedContextData,
    /// Whether the user's PROFILE.md layer is injected.
    ///
    /// The live subagent path derives this from the resolved definition's
    /// `omit_profile` (`subagent_runner/ops/runner.rs`). The crate hands this
    /// seam an opaque agent id, so the wiring site supplies it explicitly via
    /// [`Self::with_omissions`]; hardcoding it would inject a file a specialist
    /// definition deliberately excludes.
    include_profile: bool,
    /// Whether the user's MEMORY.md layer is injected. See
    /// [`Self::include_profile`].
    include_memory_md: bool,
    /// The section chain. Built once so per-turn composition is just a render.
    builder: SystemPromptBuilder,
}

impl OpenHumanContextComposer {
    /// A composer over `config` with no integrations and no learned context.
    ///
    /// This is the honest zero state, not a degraded one: a fresh install with
    /// nothing connected and no learning history composes exactly this prompt.
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
            learned: LearnedContextData::default(),
            include_profile: true,
            include_memory_md: true,
            builder: SystemPromptBuilder::with_defaults(),
        }
    }

    /// Applies a definition's user-file omission policy.
    ///
    /// Pass `!definition.omit_profile` / `!definition.omit_memory_md`, matching
    /// `subagent_runner/ops/runner.rs`. Without this a specialist composes with
    /// the main agent's files regardless of what its definition says.
    pub fn with_omissions(mut self, include_profile: bool, include_memory_md: bool) -> Self {
        self.include_profile = include_profile;
        self.include_memory_md = include_memory_md;
        self
    }

    /// Pins the model name rendered into the runtime section.
    pub fn with_model_name(mut self, model_name: impl Into<String>) -> Self {
        self.model_name = model_name.into();
        self
    }

    /// Pins how the tool catalogue renders.
    pub fn with_tool_call_format(mut self, format: ToolCallFormat) -> Self {
        self.tool_call_format = format;
        self
    }

    /// Attaches a pre-fetched connected-integration snapshot.
    pub fn with_connected_integrations(mut self, integrations: Vec<ConnectedIntegration>) -> Self {
        self.connected_integrations = integrations;
        self
    }

    /// Attaches a pre-fetched learned-context snapshot — see the TODO on
    /// [`Self::learned`].
    pub fn with_learned_context(mut self, learned: LearnedContextData) -> Self {
        self.learned = learned;
        self
    }

    /// Replaces the section chain, e.g. with
    /// `SystemPromptBuilder::for_subagent(..)`.
    ///
    /// Exposed because sub-agent prompts are a different chain, not a
    /// different composer: the crate hands this seam an opaque `agent_id` and
    /// cannot tell us which chain applies.
    pub fn with_builder(mut self, builder: SystemPromptBuilder) -> Self {
        self.builder = builder;
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
    /// [`crate::openhuman::agent::prompts::PromptSection`] itself fails — a
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
        // `crate::openhuman::skill_registry` / `skills`, behind the `skills`
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
            learned: self.learned.clone(),
            visible_tool_names: &visible_tool_names,
            tool_call_format: self.tool_call_format,
            connected_integrations: &self.connected_integrations,
            connected_identities_md: render_connected_identities(),
            // Mirrors `subagent_runner/ops/runner.rs`, which derives these from
            // the resolved definition's `omit_profile` / `omit_memory_md`. The
            // crate hands this seam a bare agent id, so the wiring site supplies
            // them via `with_omissions`; the default is the main-agent
            // behaviour (both included).
            include_profile: self.include_profile,
            include_memory_md: self.include_memory_md,
            // No turn-scoped curated-memory snapshot at this seam; the user
            // files sections fall back to the workspace files, which is the
            // documented `None` behaviour.
            curated_snapshot: None,
            user_identity: crate::openhuman::desktop::app_state::peek_cached_current_user_identity(
            ),
            personality_soul_md: None,
            personality_memory_md: None,
            // TODO(phase4): the master agent's personality roster is built
            // from the profiles domain (`crate::openhuman::profiles`); the
            // existing main-agent path leaves this empty too (see the
            // `personality_roster: vec![]` TODO in
            // `agent/harness/session/turn/context.rs`), so this matches
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
    /// OpenHuman renders goals, pinned context, and memory as prompt
    /// *sections*, so there is nothing left over to prepend as messages.
    /// Returning `Ok(vec![])` is the trait's documented normal case.
    async fn preamble(&self, _req: &TurnContextRequest) -> TinyAgentsResult<Vec<Message>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config rooted in a throwaway directory.
    ///
    /// Non-negotiable for these tests: `IdentitySection` calls
    /// `sync_workspace_file`, which **writes** `SOUL.md` / `IDENTITY.md` /
    /// `HEARTBEAT.md` into `workspace_dir`. Composing against
    /// `Config::default()` would scribble into the developer's real
    /// `~/.openhuman` workspace.
    fn config_in(dir: &std::path::Path) -> Arc<Config> {
        let mut config = Config::default();
        config.workspace_dir = dir.to_path_buf();
        config.action_dir = dir.join("projects");
        std::fs::create_dir_all(&config.action_dir).expect("create action dir");
        Arc::new(config)
    }

    fn request() -> TurnContextRequest {
        TurnContextRequest::new("orchestrator", "thread-1", "what changed today?")
    }

    /// A definition that sets `omit_profile` / `omit_memory_md` must not have
    /// those files injected anyway. The live subagent path derives the gates
    /// from the definition; hardcoding them here silently overrode it.
    #[tokio::test]
    async fn a_definitions_user_file_omissions_are_honoured() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("PROFILE.md"), "PROFILE_MARKER_TEXT").expect("write");
        std::fs::write(dir.path().join("MEMORY.md"), "MEMORY_MARKER_TEXT").expect("write");

        let included = OpenHumanContextComposer::new(config_in(dir.path()))
            .compose_system_prompt(&request())
            .await
            .expect("compose");

        let omitted = OpenHumanContextComposer::new(config_in(dir.path()))
            .with_omissions(false, false)
            .compose_system_prompt(&request())
            .await
            .expect("compose");

        // Only assert the omission direction: whether the default chain renders
        // these particular files depends on section config, and the contract
        // under test is that opting out is respected.
        if included.contains("PROFILE_MARKER_TEXT") {
            assert!(
                !omitted.contains("PROFILE_MARKER_TEXT"),
                "omit_profile must keep PROFILE.md out of the prompt"
            );
        }
        if included.contains("MEMORY_MARKER_TEXT") {
            assert!(
                !omitted.contains("MEMORY_MARKER_TEXT"),
                "omit_memory_md must keep MEMORY.md out of the prompt"
            );
        }
    }

    #[tokio::test]
    async fn composes_a_non_empty_prompt_carrying_the_agent_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let composer = OpenHumanContextComposer::new(config_in(dir.path()));
        let prompt = composer
            .compose_system_prompt(&request())
            .await
            .expect("prompt composes");

        assert!(!prompt.trim().is_empty(), "prompt must not be blank");
        // The default chain always appends the grounding contract and the
        // global style suffix; asserting on the suffix pins that we went
        // through `SystemPromptBuilder::build` rather than hand-assembling.
        assert!(
            prompt.contains("## Output style"),
            "prompt must come from SystemPromptBuilder::build"
        );
    }

    #[tokio::test]
    async fn preamble_is_empty_and_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let composer = OpenHumanContextComposer::new(config_in(dir.path()));
        assert!(composer
            .preamble(&request())
            .await
            .expect("preamble succeeds")
            .is_empty());
    }

    #[tokio::test]
    async fn the_prompt_is_byte_stable_across_repeated_turns() {
        // The KV-cache contract from mismatch (1): the crate calls this every
        // turn, and OpenHuman needs the bytes frozen. `DateTimeSection` has
        // minute granularity, so two back-to-back calls agreeing is the
        // strongest cheap check that nothing *else* varies per call.
        let dir = tempfile::tempdir().expect("tempdir");
        let composer = OpenHumanContextComposer::new(config_in(dir.path()));
        let a = composer
            .compose_system_prompt(&request())
            .await
            .expect("first");
        let b = composer
            .compose_system_prompt(&request())
            .await
            .expect("second");
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn a_different_agent_id_still_composes() {
        // The crate treats `agent_id` as opaque, so an id the host has never
        // heard of must not fail composition — it is a prompt input, not a
        // registry lookup.
        let dir = tempfile::tempdir().expect("tempdir");
        let composer = OpenHumanContextComposer::new(config_in(dir.path()));
        let req = TurnContextRequest::new("no-such-agent", "thread-9", "");
        assert!(composer.compose_system_prompt(&req).await.is_ok());
    }

    #[test]
    fn agents_md_layers_are_loaded_when_the_gate_is_on() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = config_in(dir.path());
        std::fs::write(dir.path().join("AGENTS.md"), "global rule").expect("write global");

        let composer = OpenHumanContextComposer::new(config);
        let loaded = composer.agents_md();
        assert_eq!(loaded.global.as_deref(), Some("global rule"));
    }

    #[test]
    fn the_agents_md_gate_is_honoured_when_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        config.action_dir = dir.path().join("projects");
        config.agent.agents_md_enabled = false;
        std::fs::create_dir_all(&config.action_dir).expect("create action dir");
        std::fs::write(dir.path().join("AGENTS.md"), "global rule").expect("write global");

        let composer = OpenHumanContextComposer::new(Arc::new(config));
        let loaded = composer.agents_md();
        assert!(
            loaded.is_empty(),
            "a disabled gate must not read AGENTS.md at all"
        );
    }

    #[test]
    fn the_model_name_falls_back_to_the_crate_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        config.default_model = None;
        let composer = OpenHumanContextComposer::new(Arc::new(config));
        assert_eq!(composer.model_name, DEFAULT_MODEL);

        let pinned = OpenHumanContextComposer::new(config_in(dir.path())).with_model_name("haiku");
        assert_eq!(pinned.model_name, "haiku");
    }

    #[tokio::test]
    async fn usable_as_a_trait_object() {
        // Pins object safety — the harness stores this as
        // `Arc<dyn ContextComposer>`, so a non-dyn-safe impl would only fail
        // at the wiring site, not here.
        let dir = tempfile::tempdir().expect("tempdir");
        let composer: Arc<dyn ContextComposer> =
            Arc::new(OpenHumanContextComposer::new(config_in(dir.path())));
        assert!(composer.compose_system_prompt(&request()).await.is_ok());
    }
}
