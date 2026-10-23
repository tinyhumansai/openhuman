//! What the host keeps of command policy: the parser for the model's declared
//! category.
//!
//! The autonomy tier's gate table, classification, the allowlist, the structural defences, the protected-path
//! floor and the risk score live in the TinySecurity module; see
//! `native_commands.rs` for how the host asks it.

use super::types::SecurityPolicy;
use tinysecurity_bus::CommandClass;

impl SecurityPolicy {
    /// Parse an LLM-declared command category. This is an **escalate-only**
    /// hint: callers pass it to the module, which combines it with its own
    /// classification (`max(classified, declared)`), so the model can *raise* the gate
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
