//! Shell command gating, answered by the TinySecurity module.
//!
//! The module classifies a command, applies the autonomy tier, the hidden
//! execution defence, the protected-path floor, the legacy allowlist and the
//! hourly budget. This file only translates the host's settings into the
//! module's immutable [`CommandPolicy`] and projects its stable, content-free
//! answer back into the host's tool-facing vocabulary (a [`GateDecision`], a
//! [`CommandClass`], and the existing policy-blocked messages). It decides
//! nothing itself and has no local classifier to fall back on: a module that is
//! missing, untrusted, faulted or slow denies the command.
//!
//! Embedders and tests that cannot load the native module install a
//! [`CommandGateway`] on the policy; the default is the module.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tinybox_bus::CommandClass;
use tinytools::status::POLICY_BLOCKED_MARKER;

use super::types::{AutonomyLevel, GateDecision, SecurityPolicy};

/// Which host integration runs the command. The module applies a different
/// admission rule to each, fixed by the trusted host and not by the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandFlow {
    /// The agent harness: class gate plus hidden-execution defence, with the
    /// human approval round trip made by the caller.
    Harness,
    /// Scheduled and legacy execution: the curated allowlist plus the
    /// configured risk restrictions.
    Scheduled,
}

/// When in a command's life the question is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandStage {
    /// Before the command is approved or admitted: the answer never spends the
    /// hourly budget.
    Authorize,
    /// Immediately before the command runs: an allowed command spends one
    /// action of the scope's hourly budget.
    Execute,
}

/// One question about one command.
#[derive(Clone, Copy, Debug)]
pub struct CommandQuery<'a> {
    pub command: &'a str,
    /// The model's own category for the command. It can only raise the class.
    pub declared: Option<CommandClass>,
    pub flow: CommandFlow,
    pub stage: CommandStage,
}

/// The module's answer in host terms. A refusal is an `Err` carrying the
/// agent-facing message, so a `Block` never needs a second look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandVerdict {
    pub class: CommandClass,
    /// `Allow` or `Prompt`; the caller parks for approval on `Prompt`.
    pub gate: GateDecision,
}

/// Answers command questions for a policy. The default is the native module;
/// a host that cannot load it (an embedder with its own admission service, a
/// unit test) supplies another. An implementation must fail closed.
#[async_trait]
pub trait CommandGateway: std::fmt::Debug + Send + Sync {
    async fn decide(
        &self,
        policy: &SecurityPolicy,
        query: &CommandQuery<'_>,
    ) -> Result<CommandVerdict, Refusal>;
}

/// A shared gateway handle, as stored on the policy.
pub type SharedCommandGateway = Arc<dyn CommandGateway>;

impl SecurityPolicy {
    /// Ask whether `query.command` may run, and whether a human must approve it.
    pub async fn check_command(&self, query: CommandQuery<'_>) -> Result<CommandVerdict, Refusal> {
        match &self.command_gateway {
            Some(gateway) => gateway.decide(self, &query).await,
            None => self.native_command_verdict(&query).await,
        }
    }

    /// The tier's gate for an acting call of `class`: the module's answer to a
    /// command of that class. `Err` is a refusal.
    pub async fn command_class_gate(&self, class: CommandClass) -> Result<GateDecision, Refusal> {
        match &self.command_gateway {
            Some(gateway) => gateway
                .decide(self, &class_query(class))
                .await
                .map(|verdict| verdict.gate),
            None => self.native_class_gate(class).await,
        }
    }

    /// [`Self::command_class_gate`] for the synchronous tool traits
    /// (`FsGate`, `NetGate`) that cannot be `async`. Goes through the shared
    /// sync-to-async seam; any failure is a refusal.
    pub fn command_class_gate_blocking(
        &self,
        class: CommandClass,
    ) -> Result<GateDecision, Refusal> {
        if let Some(gateway) = self.command_gateway.clone() {
            let policy = self.clone();
            return block_on_gateway(async move {
                gateway
                    .decide(&policy, &class_query(class))
                    .await
                    .map(|verdict| verdict.gate)
            });
        }
        self.native_class_gate_blocking(class)
    }
}

/// A class-only question: the gate for a command of `class`, with no command
/// text to classify.
fn class_query(class: CommandClass) -> CommandQuery<'static> {
    CommandQuery {
        command: "",
        declared: Some(class),
        flow: CommandFlow::Harness,
        stage: CommandStage::Authorize,
    }
}

/// Drive a gateway future from a synchronous caller without parking the
/// caller's runtime: a dedicated thread with its own current-thread runtime.
fn block_on_gateway<T: Send + 'static>(
    future: impl std::future::Future<Output = Result<T, Refusal>> + Send + 'static,
) -> Result<T, Refusal> {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Refusal::Unavailable)?
            .block_on(future)
    })
    .join()
    .map_err(|_| Refusal::Unavailable)?
}

/// The agent-facing message for a refused command.
pub(crate) fn refusal_message(denial: Refusal) -> String {
    match denial {
        Refusal::Protected => {
            format!("{POLICY_BLOCKED_MARKER} Command references a protected path")
        }
        Refusal::ReadOnly => format!(
            "{POLICY_BLOCKED_MARKER} Security policy: read-only mode — only read commands are \
             permitted. Do not retry this command; use a read-only approach or report that it \
             cannot be done in this mode."
        ),
        Refusal::Hidden => format!(
            "{POLICY_BLOCKED_MARKER} Command blocked: command/process substitution ($(…), \
             <(…)), backticks, and background (&) are not allowed in this mode — they can run \
             a hidden command the approval prompt wouldn't show. Plain redirects like `2>&1` \
             are fine. Do not retry as-is; rewrite the command without these constructs."
        ),
        Refusal::NotAllowed => format!(
            "{POLICY_BLOCKED_MARKER} Command not allowed by security policy. Do not retry this \
             command; it is off the allowlist for this mode."
        ),
        Refusal::HighRisk => format!(
            "{POLICY_BLOCKED_MARKER} Command blocked: high-risk command is disallowed by policy. \
             Do not retry this command; choose a safer approach or report that it cannot be done."
        ),
        Refusal::RateLimit => "Rate limit exceeded: action budget exhausted".to_owned(),
        Refusal::Category => format!(
            "{POLICY_BLOCKED_MARKER} Security policy: this command's category is not permitted in \
             the current access tier."
        ),
        Refusal::Unavailable => format!(
            "{POLICY_BLOCKED_MARKER} TinySecurity command authorization unavailable; command denied"
        ),
    }
}

/// Why a command was refused, in the host's terms. Its `Display` is the
/// agent-facing message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Protected,
    ReadOnly,
    Hidden,
    NotAllowed,
    HighRisk,
    RateLimit,
    Category,
    /// The module could not answer: missing, untrusted, faulted or too slow.
    Unavailable,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&refusal_message(*self))
    }
}

impl std::error::Error for Refusal {}

impl SecurityPolicy {
    /// The settings the module enforces for this policy. Derived entirely from
    /// host state; the request supplies none of it.
    #[cfg(feature = "security-module")]
    pub(crate) fn native_command_policy(
        &self,
        flow: CommandFlow,
        stage: CommandStage,
    ) -> Result<tinysecurity_bus::CommandPolicy, Refusal> {
        use tinysecurity_bus::{CommandAutonomy, CommandExecutionMode, ShellDialect};

        let action_dir = absolute(&self.action_dir)?;
        Ok(tinysecurity_bus::CommandPolicy {
            enabled: self.enabled,
            execution_mode: match flow {
                CommandFlow::Harness => CommandExecutionMode::HarnessGated,
                CommandFlow::Scheduled => CommandExecutionMode::Allowlisted,
            },
            shell_dialect: if cfg!(windows) {
                ShellDialect::Cmd
            } else {
                ShellDialect::Posix
            },
            autonomy: match self.autonomy {
                AutonomyLevel::ReadOnly => CommandAutonomy::ReadOnly,
                AutonomyLevel::Supervised => CommandAutonomy::Supervised,
                AutonomyLevel::Full => CommandAutonomy::Full,
            },
            allowed_commands: self.allowed_commands.clone(),
            // The authorize stage asks before a command is approved and again
            // before it runs; only the run reserves budget, so a command is
            // never counted twice.
            max_actions_per_hour: match stage {
                CommandStage::Authorize => u32::MAX,
                CommandStage::Execute => self.max_actions_per_hour,
            },
            require_approval_for_medium_risk: self.require_approval_for_medium_risk,
            block_high_risk_commands: self.block_high_risk_commands,
            action_dir,
            home_dir: dirs::home_dir().filter(|home| home.is_absolute()),
        })
    }

    #[cfg(feature = "security-module")]
    async fn native_command_verdict(
        &self,
        query: &CommandQuery<'_>,
    ) -> Result<CommandVerdict, Refusal> {
        use crate::modules::security::command::{command_decision, CommandOp};

        let policy = self.native_command_policy(query.flow, query.stage)?;
        let decision = command_decision(
            &self.workspace_dir,
            policy,
            query.command,
            query.declared.map(to_bus_class),
            CommandOp::Check,
        )
        .await;
        project(query, false, decision)
    }

    /// A class-only question needs neither an admission nor a budget.
    #[cfg(feature = "security-module")]
    async fn native_class_gate(&self, class: CommandClass) -> Result<GateDecision, Refusal> {
        use crate::modules::security::command::{command_decision, CommandOp};

        let query = class_query(class);
        let policy = self.native_command_policy(query.flow, query.stage)?;
        let decision = command_decision(
            &self.workspace_dir,
            policy,
            "",
            Some(to_bus_class(class)),
            CommandOp::Classify,
        )
        .await;
        project(&query, true, decision).map(|verdict| verdict.gate)
    }

    #[cfg(feature = "security-module")]
    fn native_class_gate_blocking(&self, class: CommandClass) -> Result<GateDecision, Refusal> {
        use crate::modules::security::command::{command_decision_blocking, CommandOp};

        let query = class_query(class);
        let policy = self.native_command_policy(query.flow, query.stage)?;
        let decision = command_decision_blocking(
            &self.workspace_dir,
            policy,
            "",
            Some(to_bus_class(class)),
            CommandOp::Classify,
        );
        project(&query, true, decision).map(|verdict| verdict.gate)
    }

    #[cfg(not(feature = "security-module"))]
    async fn native_class_gate(&self, _class: CommandClass) -> Result<GateDecision, Refusal> {
        Err(Refusal::Unavailable)
    }

    #[cfg(not(feature = "security-module"))]
    async fn native_command_verdict(
        &self,
        _query: &CommandQuery<'_>,
    ) -> Result<CommandVerdict, Refusal> {
        Err(Refusal::Unavailable)
    }

    #[cfg(not(feature = "security-module"))]
    fn native_class_gate_blocking(&self, _class: CommandClass) -> Result<GateDecision, Refusal> {
        Err(Refusal::Unavailable)
    }
}

#[cfg(feature = "security-module")]
fn absolute(path: &std::path::Path) -> Result<PathBuf, Refusal> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|_| Refusal::Unavailable)
}

#[cfg(feature = "security-module")]
fn to_bus_class(class: CommandClass) -> tinysecurity_bus::CommandClass {
    use tinysecurity_bus::CommandClass as Bus;
    match class {
        CommandClass::Read => Bus::Read,
        CommandClass::Write => Bus::Write,
        CommandClass::Network => Bus::Network,
        CommandClass::Install => Bus::Install,
        CommandClass::Destructive => Bus::Destructive,
    }
}

#[cfg(feature = "security-module")]
fn from_bus_class(class: tinysecurity_bus::CommandClass) -> CommandClass {
    use tinysecurity_bus::CommandClass as Bus;
    match class {
        Bus::Read => CommandClass::Read,
        Bus::Write => CommandClass::Write,
        Bus::Network => CommandClass::Network,
        Bus::Install => CommandClass::Install,
        Bus::Destructive => CommandClass::Destructive,
    }
}

/// The host's gate for a module gate.
#[cfg(feature = "security-module")]
pub(crate) fn host_gate(gate: tinysecurity_bus::CommandGate) -> GateDecision {
    use tinysecurity_bus::CommandGate;
    match gate {
        CommandGate::Allow => GateDecision::Allow,
        CommandGate::Prompt => GateDecision::Prompt,
        CommandGate::Block => GateDecision::Block,
    }
}

/// Project the module's content-free decision into the host contract.
///
/// `class_only` marks a question about a tier's gate for a class of command
/// rather than about one command.
#[cfg(feature = "security-module")]
pub(crate) fn project(
    query: &CommandQuery<'_>,
    class_only: bool,
    decision: Result<tinysecurity_bus::CommandDecision, String>,
) -> Result<CommandVerdict, Refusal> {
    use tinysecurity_bus::{CommandDenial, CommandGate};

    let decision = match decision {
        Ok(decision) => decision,
        Err(error) => {
            log::warn!("[security][command] module unavailable; command denied: {error}");
            return Err(Refusal::Unavailable);
        }
    };
    if let Some(denial) = decision.denial {
        let refusal = match denial {
            CommandDenial::Protected => Refusal::Protected,
            CommandDenial::ReadOnly => match query.flow {
                CommandFlow::Harness => Refusal::ReadOnly,
                CommandFlow::Scheduled => Refusal::NotAllowed,
            },
            CommandDenial::Command => match query.flow {
                CommandFlow::Harness => Refusal::Hidden,
                CommandFlow::Scheduled => Refusal::NotAllowed,
            },
            CommandDenial::HighRisk => Refusal::HighRisk,
            CommandDenial::RateLimit => Refusal::RateLimit,
        };
        log::debug!(
            "[security][command] refused flow={:?} reason={refusal:?}",
            query.flow
        );
        return Err(refusal);
    }
    // A class question reports a blocked tier as such; for a real command a
    // block always carries a denial, so one without is refused outright.
    if decision.gate == CommandGate::Block && !class_only {
        return Err(Refusal::Category);
    }
    let gate = host_gate(decision.gate);
    Ok(CommandVerdict {
        class: from_bus_class(decision.class),
        gate,
    })
}

#[cfg(test)]
#[path = "native_commands_tests.rs"]
mod tests;
