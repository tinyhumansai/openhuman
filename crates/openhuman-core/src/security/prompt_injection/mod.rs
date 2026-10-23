//! Prompt injection screening and enforcement.
//!
//! This module centralizes prompt-injection checks so user-provided prompts and
//! untrusted content can be screened before any model or tool execution path. The
//! rules, weights and thresholds are the TinySecurity module's (`ScanPrompt2`,
//! `ScanToolDefinition`); the host asks through the [`PromptScanner`] port and
//! enforces the answer.

mod enforce;
mod module_scanner;
mod scanner;
#[cfg(test)]
pub(crate) mod scanner_stub;
mod types;

pub use enforce::enforce_prompt_input;
#[cfg(test)]
pub(crate) use scanner::tests::{current_scoped, with_scanner};
pub use scanner::{
    screen, screen_tool_definition, windows, DefinitionScan, DefinitionVerdict, PromptScan,
    PromptScanner, Tier, CALL_DEADLINE, MAX_SCANNED_BYTES, OVERLAP_BYTES, WINDOW_BYTES,
};
pub use types::{
    PromptEnforcementAction, PromptEnforcementContext, PromptEnforcementDecision,
    PromptInjectionReason, PromptInjectionVerdict,
};
