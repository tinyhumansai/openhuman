//! The live OpenHuman composition over `tinyagents_runtime::Session`.
//!
//! Host hooks carry product state; `tinyagents_runtime` owns history,
//! transcripts, prefixes, snapshots, resume, and persistence.

mod bound_history;
mod compact;
pub use compact::{CompactOptions, CompactOutcome};
#[path = "runtime_session_enrichment.rs"]
mod enrichment;
pub(super) mod memory_block;
mod permanent;
mod post_commit;
#[path = "runtime_session/prelude.rs"]
mod prelude;
#[path = "runtime_session/prompt.rs"]
mod prompt;
mod prompt_refresh;
#[path = "runtime_session/prompt_surface.rs"]
mod prompt_surface;
#[path = "runtime_session_usage.rs"]
mod usage;
pub(super) use usage::{account_committed_turn_against_goal, holistic_last_turn_usage};
mod identity;
#[path = "../../memory/lifecycle/session_hooks.rs"]
mod session_hooks;
mod tool_rules;
#[path = "runtime_session_turn.rs"]
mod turn;
pub(super) use turn::begin_turn_resume;

type ActiveTerminalSidecar = std::sync::Arc<
    std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
>;

use std::sync::Arc;

use anyhow::Result;
use tinyagents_harness::multimodal::markers::extract_image_placeholders_in_text;
use tinyagents_harness::tool::ToolSnapshot;
use tinyagents_runtime::{
    CommitReceipt, ResumePreparation, SessionBuilder, SessionTerminal, TranscriptTarget,
    TurnPreparation,
};
use tinyinference_llm::message::Message;

use super::types::OpenHumanSessionHost;
use crate::agent::{
    message_convert::{user_message_from_text, user_text_with_markers},
    session_host::{
        driver::OpenHumanSessionDriver, OpenHumanSessionHooks, OpenHumanTranscriptCodec,
    },
    tinyagents::{host::OpenHumanRunContext, middleware::turn_context::TurnContextMiddleware},
};
use crate::integrations::announcement_notes::{
    integration_announcement_note, mcp_announcement_note, skill_announcement_note,
    skill_retraction_note,
};

#[path = "runtime_session_progress.rs"]
mod progress;

/// Mutable product state observed by the runtime hooks.
///
/// This type has no message accumulator, raw transcript rows, prefix matching,
/// resume cache, or persistence handle. Those are exclusively `Session` state.
#[derive(Default)]
pub(super) struct OpenHumanSessionState {
    terminals: Vec<SessionTerminal>,
    pub(super) last_turn_hit_cap: bool,
    pub(super) last_turn_usage: Option<crate::agent::tinyagents::host::LastTurnUsage>,
    pub(super) last_turn_iterations: u32,
    pub(super) context_middleware: Option<TurnContextMiddleware>,
    required_output: Option<tinyagents_harness::config::RequiredOutput>,
    pub(crate) pending_turn_overrides: super::types::TurnOverrides,
    pub(super) active_turn_overrides: super::types::TurnOverrides,
    pub(crate) pending_response_shape: super::types::PendingResponseShape,
    pub(super) reply_language_directive: Option<String>,
    pub(super) time_zone: Option<String>,
    prelude: Option<OpenHumanTurnPrelude>,
    active_terminal_sidecar: Option<ActiveTerminalSidecar>,
    /// Previous turn's deferred goal accounting, awaited before the next turn.
    pending_post_commit: Option<tokio::task::JoinHandle<()>>,
}

/// Owned host-only inputs used by the async runtime preparation hook.
/// It deliberately excludes transcript/history/tool snapshots, which belong to
/// `tinyagents_runtime::Session`.
#[derive(Clone)]
struct OpenHumanTurnPrelude {
    config: crate::config::AgentConfig,
    /// Host prompt/utilisation state, shared by the runtime hooks. It is not
    /// generic conversation state and the runtime never persists it.
    prompt_builder: Arc<crate::agent::prompts::SystemPromptBuilder>,
    context_config: crate::config::ContextConfig,
    tool_policy: Arc<
        dyn tinyagents_harness::middleware::ToolCallPolicy<
            crate::agent::tinyagents::host::OpenHumanRunContext,
        >,
    >,
    pack_registry_handle: Option<tinyagents_harness_tools::skills::PackRegistryHandle>,
    tool_dispatcher: Arc<dyn tinytools_agent::dialect::ToolDialect>,
    workspace_dir: std::path::PathBuf,
    action_dir: std::path::PathBuf,
    model_name: String,
    agent_definition_name: String,
    /// No per-turn memory pack for this agent (definition's
    /// `omit_memory_context`); its turns are still logged.
    omit_memory_context: bool,
    thread_id: Option<String>,
    agent_definition_id: String,
    event_session_id: String,
    event_channel: String,
    subagent_tool_ceiling_names: std::collections::HashSet<String>,
    turn_model_source: crate::agent::tinyagents::TurnModelSource,
    temperature: f64,
    workspace_descriptor: Option<tinytools::WorkspaceDescriptor>,
    session_key: String,
    session_parent_prefix: Option<String>,
    on_progress: Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
    run_queue:
        Option<Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    host_env: crate::agent::turn_env::HostTurnEnv,
    allowed_subagent_ids: std::collections::HashSet<String>,
    sandbox_mode: tinyagents_definition::SandboxMode,
    definition: Option<Arc<crate::agent_registry::host_definition::AgentDefinition>>,
    runtime_config: Option<Arc<crate::config::Config>>,
    /// This session's tool-rule layers; see `tool_rules.rs`.
    tool_rules: Arc<tinytools::ToolRuleSet>,
    /// The one authoritative, request-refreshable composition of executable
    /// tools, policy, and provider schema. Generic runtime owns the immutable
    /// `ToolSnapshot`; this host surface is the source used to create it.
    tool_surface: Arc<std::sync::Mutex<OpenHumanTurnToolSurface>>,
    mutable: Arc<std::sync::Mutex<OpenHumanTurnPreludeMutable>>,
}

/// Host-owned tool composition from which one runtime request is prepared.
/// All three views are rebuilt together so a prompt schema, execution source,
/// and fail-closed policy cannot describe different authority.
struct OpenHumanTurnToolSurface {
    tools: Arc<Vec<Box<dyn tinytools::Tool>>>,
    synthesized_tools: Arc<Vec<Box<dyn tinytools::Tool>>>,
    tool_specs: Arc<Vec<Arc<tinytools::ToolSpec>>>,
    durable_tool_specs: Arc<Vec<Arc<tinytools::ToolSpec>>>,
    visible_tool_specs: Arc<Vec<Arc<tinytools::ToolSpec>>>,
    visible_tool_names: std::collections::HashSet<String>,
    /// Registered but never advertised: the `Deferred` tools the harness's
    /// `tool_search` bridge can reach. Part of the snapshot the driver treats
    /// as the final allowlist, and classified `Allow` by the policy, so a
    /// found tool is callable. See `OpenHumanSessionHost::deferred_tool_names`.
    deferred_tool_names: std::collections::HashSet<String>,
    permanent_tool_names: std::collections::HashSet<String>,
    /// Whether this belt reaches deferred tools at all; fixed at build.
    discovery_enabled: bool,
    /// The definition's own `deferred_tools`; see `meta::deferred_set`.
    requested_deferred_tools: Arc<[String]>,
    /// Whether newly connected delegates may enter the visible belt without a
    /// caller explicitly allowing them. A hide/named restriction turns this
    /// off so refresh cannot reopen withdrawn authority.
    auto_include_new_synthesized_tools: bool,
    synthesized_tool_names: std::collections::HashSet<String>,
    tool_policy_session: tinytools::ToolPolicySession,
    event_session_id: String,
    event_channel: String,
    agent_definition_name: String,
}

/// Per-session product observations formerly scattered across the legacy
/// `core_turn` loop. Generic history, raw transcript and prefix state are not here.
#[path = "runtime_session/announcement_state.rs"]
mod announcement_state;
use announcement_state::OpenHumanTurnPreludeMutable;

#[path = "runtime_session/ensure.rs"]
mod ensure;

impl OpenHumanSessionHost {
    pub(super) fn update_runtime_prelude_progress(
        &mut self,
        tx: Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
    ) {
        if let Some(prelude) = self
            .runtime_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prelude
            .as_mut()
        {
            prelude.on_progress = tx;
        }
    }

    /// Seed a cold runtime session from a host-provided message log.
    ///
    /// The runtime receives both the seed and any subsequent append; the host
    /// does not retain a parallel history cache. This lower-fidelity fallback
    /// intentionally has no raw transcript rows because its source is a
    /// role/content log rather than an authoritative transcript.
    pub fn seed_resume_from_messages(
        &mut self,
        mut messages: Vec<(String, String)>,
        current_user_message: &str,
    ) -> Result<()> {
        if self.runtime_session.is_some() {
            return Ok(());
        }
        if messages.last().is_some_and(|(role, text)| {
            role == "user" && text.trim() == current_user_message.trim()
        }) {
            messages.pop();
        }
        if messages.is_empty() {
            return Ok(());
        }
        self.ensure_runtime_session()?;
        let history = messages
            .into_iter()
            .map(|(role, text)| match role.as_str() {
                "system" => Message::system(text),
                "assistant" | "agent" => Message::assistant(text),
                _ => Message::user(text),
            })
            .collect();
        self.runtime_session
            .as_mut()
            .expect("runtime session initialized")
            .seed_history(history, Vec::new())
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    /// Dispatch one public OpenHuman turn through the neutral runtime.
    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
        self.turn_with_origin(user_message, None).await
    }

    /// Publish a caller-driven visibility/policy mutation into the live hook
    /// state. The runtime keeps generic snapshots immutable per request; this
    /// simply replaces the host composition from which the *next* snapshot,
    /// execution source, and enforcement object are built.
    pub(in crate::agent::session_host) fn sync_runtime_tool_surface(&self) {
        self.sync_runtime_tool_surface_with_auto_include(None);
    }

    /// Publish a visibility mutation and record whether future dynamically
    /// connected delegates may enter the surface without an explicit allow.
    pub(in crate::agent::session_host) fn sync_runtime_tool_surface_with_auto_include(
        &self,
        auto_include_new_synthesized_tools: Option<bool>,
    ) {
        let prelude = self
            .runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .prelude
            .clone();
        let Some(prelude) = prelude else {
            return;
        };
        let prior_auto_include = prelude
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .auto_include_new_synthesized_tools;
        prelude.replace_tool_surface(OpenHumanTurnToolSurface {
            tools: self.tools.clone(),
            synthesized_tools: self.synthesized_tools.clone(),
            tool_specs: self.tool_specs.clone(),
            durable_tool_specs: self.durable_tool_specs.clone(),
            visible_tool_specs: self.visible_tool_specs.clone(),
            visible_tool_names: self.visible_tool_names.clone(),
            deferred_tool_names: self.deferred_tool_names.clone(),
            permanent_tool_names: self.permanent_tool_names.clone(),
            discovery_enabled: self.discovery_enabled,
            requested_deferred_tools: self.requested_deferred_tools.clone(),
            auto_include_new_synthesized_tools: auto_include_new_synthesized_tools
                .unwrap_or(prior_auto_include),
            synthesized_tool_names: self.synthesized_tool_names.clone(),
            tool_policy_session: self.tool_policy_session.clone(),
            event_session_id: self.event_session_id.clone(),
            event_channel: self.event_channel.clone(),
            agent_definition_name: self.agent_definition_name.clone(),
        });
    }
}

#[path = "runtime_session_tool_surface.rs"]
pub(super) mod tool_surface;

#[cfg(test)]
#[path = "runtime_session_juice_tests.rs"]
mod juice_tests;

#[cfg(test)]
#[path = "runtime_session_goal_tests.rs"]
mod goal_tests;

mod parent_snapshot;
#[path = "../../integrations/prelude_integrations.rs"]
mod prelude_integrations;

#[cfg(test)]
#[path = "runtime_session_enrichment_tests.rs"]
mod enrichment_tests;

#[cfg(test)]
#[path = "runtime_session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_session_prompt_tests.rs"]
mod prompt_tests;
