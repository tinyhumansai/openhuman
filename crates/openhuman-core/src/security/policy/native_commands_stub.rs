//! A scripted [`CommandGateway`] for tests that exercise what a caller does
//! with the module's answer (parking on `Prompt`, rendering a refusal, spending
//! a stage) without loading the native module. It classifies nothing: every
//! answer is the one the test scripted, and the queries it was asked are
//! recorded so a test can pin what the caller sent.
//!
//! What the module itself answers for a given command is pinned by the
//! module-backed tests in `modules/security_command_tests.rs`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinysecurity_bus::CommandClass;

use super::native_commands::{
    CommandFlow, CommandGateway, CommandQuery, CommandStage, CommandVerdict, Refusal,
};
use super::types::{GateDecision, SecurityPolicy};

/// One recorded question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Asked {
    pub command: String,
    pub declared: Option<CommandClass>,
    pub flow: CommandFlow,
    pub stage: CommandStage,
}

/// The autonomy tier's gate for a class of command, written out for tests: a
/// disabled policy allows everything; read-only allows only `Read`;
/// supervised prompts on every acting class; full runs `Read`/`Write` and
/// prompts on the rest. The module owns the real table.
pub(crate) fn tier_table(policy: &SecurityPolicy, class: CommandClass) -> GateDecision {
    use super::types::AutonomyLevel;
    if !policy.enabled {
        return GateDecision::Allow;
    }
    match (policy.autonomy, class) {
        (_, CommandClass::Read) => GateDecision::Allow,
        (AutonomyLevel::ReadOnly, _) => GateDecision::Block,
        (AutonomyLevel::Supervised, _) => GateDecision::Prompt,
        (AutonomyLevel::Full, CommandClass::Write) => GateDecision::Allow,
        (AutonomyLevel::Full, _) => GateDecision::Prompt,
    }
}

type Answer = Box<
    dyn Fn(&SecurityPolicy, &CommandQuery<'_>) -> Result<CommandVerdict, Refusal> + Send + Sync,
>;

pub(crate) struct StubGateway {
    answer: Answer,
    asked: Mutex<Vec<Asked>>,
}

impl std::fmt::Debug for StubGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubGateway").finish_non_exhaustive()
    }
}

impl StubGateway {
    pub(crate) fn answering(
        answer: impl Fn(&SecurityPolicy, &CommandQuery<'_>) -> Result<CommandVerdict, Refusal>
            + Send
            + Sync
            + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            answer: Box::new(answer),
            asked: Mutex::new(Vec::new()),
        })
    }

    /// Always `gate` for a command of class `class`.
    pub(crate) fn gate(class: CommandClass, gate: GateDecision) -> Arc<Self> {
        Self::answering(move |_, _| Ok(CommandVerdict { class, gate }))
    }

    pub(crate) fn allow() -> Arc<Self> {
        Self::gate(CommandClass::Read, GateDecision::Allow)
    }

    pub(crate) fn prompt() -> Arc<Self> {
        Self::gate(CommandClass::Write, GateDecision::Prompt)
    }

    pub(crate) fn refuse(reason: Refusal) -> Arc<Self> {
        Self::answering(move |_, _| Err(reason))
    }

    /// Answers a class-only question with [`tier_table`] and allows every real
    /// command. For tests of what a caller does with each gate; that the
    /// module's table is this table is pinned by the module-backed
    /// `released_module_tier_gate_matches_the_inlined_table`.
    pub(crate) fn tiered() -> Arc<Self> {
        Self::answering(|policy, query| {
            let class = query.declared.unwrap_or(CommandClass::Read);
            Ok(CommandVerdict {
                class,
                gate: if query.command.is_empty() {
                    tier_table(policy, class)
                } else {
                    GateDecision::Allow
                },
            })
        })
    }

    pub(crate) fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }
}

impl super::native_commands::Sealed for StubGateway {}

#[async_trait]
impl CommandGateway for StubGateway {
    async fn decide(
        &self,
        policy: &SecurityPolicy,
        query: &CommandQuery<'_>,
    ) -> Result<CommandVerdict, Refusal> {
        self.asked.lock().unwrap().push(Asked {
            command: query.command.to_owned(),
            declared: query.declared,
            flow: query.flow,
            stage: query.stage,
        });
        (self.answer)(policy, query)
    }
}

impl SecurityPolicy {
    /// This policy with `gateway` answering its command questions.
    pub(crate) fn with_gateway(mut self, gateway: Arc<StubGateway>) -> Self {
        self.command_gateway = Some(gateway);
        self
    }
}
