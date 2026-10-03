//! The one factory that assembles OpenHuman's TinyAgents host capabilities.
//!
//! Keeping this construction in one place makes the boundary auditable: every
//! concrete adapter receives values from the same host session and explicit
//! [`OpenHumanRunContext`], and the generic runtime receives only its canonical
//! [`tinyagents_harness::host::HostCapabilities`] bundle.

use std::sync::Arc;

use tinyagents_harness::host::{
    BudgetGate, ContextComposer, HostCapabilities, LearningSink, ModelResolver, ProgressSink,
    SecurityGate, ToolOutcomeClassifier,
};

use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::hooks::PostTurnHook;
use crate::config::Config;
use crate::security::policy::SecurityPolicy;
use crate::tools::agent_policy::ToolPolicySession;
use tinytools::Tool;

use super::{
    OpenHumanBudgetGate, OpenHumanContextComposer, OpenHumanDefinitionRegistry,
    OpenHumanLearningSink, OpenHumanModelResolver, OpenHumanProgressSink, OpenHumanRunContext,
    OpenHumanSecurityGate, OpenHumanToolOutcomeClassifier,
};

/// Runtime-owned inputs required to build every OpenHuman host capability.
///
/// This is deliberately an input value, rather than a global lookup. The
/// caller obtains it while building a session and passes it alongside the
/// matching [`OpenHumanRunContext`].
pub struct OpenHumanHostBundleInputs {
    pub config: Arc<Config>,
    pub definitions: Arc<AgentDefinitionRegistry>,
    pub security_policy: Arc<SecurityPolicy>,
    pub tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    pub tool_policy: Option<Arc<ToolPolicySession>>,
    pub post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    /// See [`OpenHumanHostBase::session_definition`].
    pub session_definition: Option<Arc<crate::agent::harness::definition::AgentDefinition>>,
}

/// Process/session-owned dependencies shared by hosted invocations.
///
/// This deliberately excludes mutable turn authority: tools, tool-policy
/// sessions, progress and the concrete capability bundle are created for each
/// [`OpenHumanHostInvocationInputs`]. Keeping the split visible prevents a
/// concurrent turn from replacing another turn's security or tool surface.
pub struct OpenHumanHostBase {
    pub config: Arc<Config>,
    pub definitions: Arc<AgentDefinitionRegistry>,
    pub security_policy: Arc<SecurityPolicy>,
    pub post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    /// The session's own caller-supplied definition, when it was built from one
    /// rather than from a registry id — see
    /// [`OpenHumanDefinitionRegistry::with_session_definition`]. Lives here, beside
    /// the registry it outranks, because it is fixed for the session's lifetime:
    /// one `Runtime` hosts several independently defined agents, but each gets
    /// its own session and therefore its own base.
    pub session_definition: Option<Arc<crate::agent::harness::definition::AgentDefinition>>,
}

/// Explicit inputs which vary for every hosted agent invocation.
pub struct OpenHumanHostInvocationInputs {
    pub base: Arc<OpenHumanHostBase>,
    pub tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    pub tool_policy: Option<Arc<ToolPolicySession>>,
    /// The current turn's already-selected model route set.
    pub model_resolver: Option<Arc<dyn ModelResolver<()>>>,
}

/// OpenHuman's concrete capability adapters plus their erased crate bundle.
/// The harness's memory and experience capabilities are left unset: memory is
/// the `memory` tool, not a harness capability.
///
/// Typed handles make it possible to verify wiring without downcasting trait
/// objects. The runtime consumes [`Self::capabilities`]; the typed fields exist
/// solely to preserve the host boundary and to support focused wiring tests.
pub struct OpenHumanHostBundle {
    pub capabilities: HostCapabilities<()>,
    pub context: Arc<OpenHumanContextComposer>,
    pub definitions: Arc<OpenHumanDefinitionRegistry>,
    pub security: Arc<OpenHumanSecurityGate>,
    pub models: Arc<OpenHumanModelResolver>,
    pub budget: Arc<OpenHumanBudgetGate>,
    pub progress: Arc<OpenHumanProgressSink>,
    pub learning: Arc<OpenHumanLearningSink>,
    pub tool_outcomes: Arc<OpenHumanToolOutcomeClassifier>,
}

/// Builds the full OpenHuman host bundle for an explicit turn.
pub struct OpenHumanHostBundleFactory;

impl OpenHumanHostBundleFactory {
    /// Builds one full capability bundle from immutable host base dependencies
    /// and this invocation's tool/security authority.
    pub fn build_for_invocation(
        inputs: OpenHumanHostInvocationInputs,
        turn: &OpenHumanRunContext,
    ) -> OpenHumanHostBundle {
        let mut bundle = Self::build(
            OpenHumanHostBundleInputs {
                config: inputs.base.config.clone(),
                definitions: inputs.base.definitions.clone(),
                security_policy: inputs.base.security_policy.clone(),
                tool_sets: inputs.tool_sets,
                tool_policy: inputs.tool_policy,
                post_turn_hooks: inputs.base.post_turn_hooks.clone(),
                session_definition: inputs.base.session_definition.clone(),
            },
            turn,
        );
        if let Some(model_resolver) = inputs.model_resolver {
            bundle.capabilities.models = model_resolver;
        }
        bundle
    }

    /// Constructs every concrete adapter from a single session input set.
    ///
    /// The run context supplies per-turn state, while `inputs` supplies durable
    /// session/runtime dependencies. No adapter is optional for OpenHuman. The
    /// progress seam is registered but unconsumed: the turn's live channel is
    /// owned by `OpenhumanEventBridge` (see below).
    pub fn build(
        inputs: OpenHumanHostBundleInputs,
        _turn: &OpenHumanRunContext,
    ) -> OpenHumanHostBundle {
        let context = Arc::new(OpenHumanContextComposer::new(Arc::clone(&inputs.config)));
        let registered_tools = Arc::new(
            inputs
                .tool_sets
                .iter()
                .flat_map(|set| set.iter())
                .map(|tool| tool.name().to_string())
                .collect(),
        );
        let deferred_tools = Arc::new(
            inputs
                .tool_sets
                .iter()
                .flat_map(|set| set.iter())
                .filter(|tool| tool.exposure() == tinytools::ToolExposure::Deferred)
                .map(|tool| tool.name().to_string())
                .collect(),
        );
        let session_delegation_tools = Arc::new(
            inputs
                .tool_sets
                .iter()
                .skip(1)
                .flat_map(|set| set.iter())
                .map(|tool| tool.name().to_string())
                .collect(),
        );
        let mut definitions_adapter = OpenHumanDefinitionRegistry::new(inputs.definitions)
            .with_config(Arc::clone(&inputs.config))
            .with_registered_tools(registered_tools)
            .with_deferred_tools(deferred_tools)
            .with_session_delegation_tools(session_delegation_tools);
        if let Some(definition) = inputs.session_definition {
            definitions_adapter = definitions_adapter.with_session_definition(definition);
        }
        let definitions = Arc::new(definitions_adapter);
        let mut security = OpenHumanSecurityGate::new(inputs.security_policy, inputs.tool_sets);
        if let Some(policy) = inputs.tool_policy {
            security = security.with_tool_policy(policy);
        }
        let security = Arc::new(security);
        let models = Arc::new(OpenHumanModelResolver::new(Arc::clone(&inputs.config)));
        let budget = Arc::new(OpenHumanBudgetGate::new(Arc::clone(&inputs.config)));
        // The turn's live `AgentProgress` channel is fed by exactly one
        // producer: `OpenhumanEventBridge`, which `turn_runner` subscribes to
        // the run's `EventSink` on every turn and which carries what the UI
        // needs (iteration attribution, thinking, tool-argument fragments,
        // sub-agent scoping, cost). The harness also mirrors the loop onto the
        // coarse host `ProgressSink` (`emit_host_progress`: `Token` per model
        // delta, `ToolCall`/`ToolCallFinished`, `Finished`), so wiring this
        // sink to the same channel delivered every delta and every tool row
        // twice and interleaved the copies in the interim bubble. The sink
        // stays registered as the host capability with an unconsumed channel;
        // nothing OpenHuman renders depends on it.
        let progress = Arc::new(OpenHumanProgressSink::new(tokio::sync::mpsc::channel(1).0));
        let learning = Arc::new(OpenHumanLearningSink::new(inputs.post_turn_hooks));
        let tool_outcomes = Arc::new(OpenHumanToolOutcomeClassifier::new());

        let capabilities = HostCapabilities::new(
            context.clone() as Arc<dyn ContextComposer>,
            definitions.clone(),
            security.clone() as Arc<dyn SecurityGate>,
            models.clone() as Arc<dyn ModelResolver<()>>,
        )
        .with_budget(budget.clone() as Arc<dyn BudgetGate>)
        .with_progress(progress.clone() as Arc<dyn ProgressSink>)
        .with_learning(learning.clone() as Arc<dyn LearningSink>)
        .with_tool_outcomes(tool_outcomes.clone() as Arc<dyn ToolOutcomeClassifier>);

        OpenHumanHostBundle {
            capabilities,
            context,
            definitions,
            security,
            models,
            budget,
            progress,
            learning,
            tool_outcomes,
        }
    }
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
