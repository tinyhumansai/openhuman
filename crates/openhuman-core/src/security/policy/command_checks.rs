//! What the host keeps of command policy: the autonomy tier's gate table, and
//! the parser for the model's declared category.
//!
//! Classification, the allowlist, the structural defences, the protected-path
//! floor and the risk score live in the TinySecurity module; see
//! `native_commands.rs` for how the host asks it.

use super::types::{AutonomyLevel, GateDecision, SecurityPolicy};
use tinybox_bus::CommandClass;

impl SecurityPolicy {
    /// The gate decision for an acting tool call of `class` under this policy's
    /// autonomy tier. The harness turns `Prompt` into an `ApprovalGate`
    /// round-trip *before* the tool runs; `Block` is refused outright.
    ///
    /// Matrix: read-only allows only `Read`; ask-before-edit (`Supervised`)
    /// prompts on every acting class; full runs `Read`/`Write` silently but
    /// always prompts on `Network`/`Destructive`.
    pub fn gate_decision(&self, class: CommandClass) -> GateDecision {
        if !self.enabled {
            return GateDecision::Allow;
        }
        match self.autonomy {
            AutonomyLevel::ReadOnly => match class {
                CommandClass::Read => GateDecision::Allow,
                _ => GateDecision::Block,
            },
            AutonomyLevel::Supervised => match class {
                CommandClass::Read => GateDecision::Allow,
                _ => GateDecision::Prompt,
            },
            AutonomyLevel::Full => match class {
                CommandClass::Read | CommandClass::Write => GateDecision::Allow,
                CommandClass::Network | CommandClass::Install | CommandClass::Destructive => {
                    GateDecision::Prompt
                }
            },
        }
    }

    /// Parse an LLM-declared command category. This is an **escalate-only**
    /// hint: callers combine it with the deterministic floor via
    /// `classify_command(cmd).max(declared)`, so the model can *raise* the gate
    /// (e.g. flag a `Write` as `Destructive` to request confirmation) but can
    /// never lower what the runtime determined. Unknown / empty → `None`.
    pub fn parse_declared_class(declared: &str) -> Option<CommandClass> {
        match declared.trim().to_ascii_lowercase().as_str() {
            "read" => Some(CommandClass::Read),
            "write" => Some(CommandClass::Write),
            "network" => Some(CommandClass::Network),
            "install" => Some(CommandClass::Install),
            "destructive" => Some(CommandClass::Destructive),
            _ => None,
        }
    }
}
