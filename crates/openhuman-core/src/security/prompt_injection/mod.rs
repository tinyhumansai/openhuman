//! Prompt injection detection and enforcement.
//!
//! This module centralizes prompt-injection checks so user-provided prompts
//! can be screened before any model or tool execution path.

mod detector;
#[cfg(test)]
mod scan_port;

#[cfg(test)]
pub(crate) use scan_port::{
    install_allowing_prompt_scan, install_prompt_scan_port, PromptScanPort,
};

pub(crate) use detector::{
    enforce_prompt_input_in, enforce_prompt_input_with_config, screening_config_in,
    tool_scan_config,
};

pub use detector::{
    enforce_prompt_input, scan_tool_definition, PromptEnforcementAction, PromptEnforcementContext,
    PromptEnforcementDecision, PromptInjectionReason, PromptInjectionVerdict,
    ToolDefinitionScanHit, MAX_PROMPT_BYTES,
};

#[cfg(test)]
#[path = "prompt_injection_tests.rs"]
mod tests;
