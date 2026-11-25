//! A scripted TinySecurity command gateway with a lexical protected-path
//! floor, for tests of what callers do with a floor refusal.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::security::policy::{
    AutonomyLevel, CommandClass, CommandGateway, CommandQuery, CommandStage, CommandVerdict,
    GateDecision, Refusal, SecurityPolicy,
};

/// Answers like the module, in miniature: the floor first (`..`, `/etc`,
/// `.ssh`), then the hidden-execution defence and the tier's class gate.
/// `rm` is destructive, `touch` a write, anything else a read.
#[derive(Debug, Default)]
pub(crate) struct FloorGateway {
    pub queries: Mutex<Vec<(String, Option<CommandClass>, CommandStage)>>,
}

impl FloorGateway {
    fn protected(command: &str) -> bool {
        command.contains("..") || command.contains("/etc") || command.contains(".ssh")
    }

    fn class(query: &CommandQuery<'_>) -> CommandClass {
        let own = match query.command.split_whitespace().next() {
            Some("rm") => CommandClass::Destructive,
            Some("touch") => CommandClass::Write,
            _ => CommandClass::Read,
        };
        query.declared.map_or(own, |declared| declared.max(own))
    }
}

impl crate::security::policy::Sealed for FloorGateway {}

#[async_trait]
impl CommandGateway for FloorGateway {
    async fn decide(
        &self,
        policy: &SecurityPolicy,
        query: &CommandQuery<'_>,
    ) -> Result<CommandVerdict, Refusal> {
        self.queries
            .lock()
            .unwrap()
            .push((query.command.to_owned(), query.declared, query.stage));
        if Self::protected(query.command) {
            return Err(Refusal::Protected);
        }
        if policy.autonomy != AutonomyLevel::Full && query.command.contains("$(") {
            return Err(Refusal::Hidden);
        }
        let class = Self::class(query);
        let gate = match (policy.autonomy, class) {
            (_, CommandClass::Read) => GateDecision::Allow,
            (AutonomyLevel::ReadOnly, _) => return Err(Refusal::ReadOnly),
            (AutonomyLevel::Supervised, _) => GateDecision::Prompt,
            (AutonomyLevel::Full, CommandClass::Write) => GateDecision::Allow,
            (AutonomyLevel::Full, _) => GateDecision::Prompt,
        };
        Ok(CommandVerdict { class, gate })
    }

    async fn protected_class(
        &self,
        _policy: &SecurityPolicy,
        query: &CommandQuery<'_>,
    ) -> Option<CommandClass> {
        Self::protected(query.command).then(|| Self::class(query))
    }
}

/// A policy at `autonomy` that asks [`FloorGateway`].
pub(crate) fn floor_policy(
    autonomy: AutonomyLevel,
) -> (SecurityPolicy, std::sync::Arc<FloorGateway>) {
    let gateway = std::sync::Arc::new(FloorGateway::default());
    let mut policy = SecurityPolicy {
        autonomy,
        ..SecurityPolicy::default()
    };
    policy.command_gateway = Some(gateway.clone());
    (policy, gateway)
}
