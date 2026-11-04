//! Prompt injection detection and enforcement.
//!
//! This module centralizes prompt-injection checks so user-provided prompts
//! can be screened before any model or tool execution path.

mod detector;

pub(crate) use detector::{enforce_prompt_input_in, enforce_prompt_input_with_config};

pub use detector::{
    enforce_prompt_input, scan_tool_definition, PromptEnforcementAction, PromptEnforcementContext,
    PromptEnforcementDecision, PromptInjectionReason, PromptInjectionVerdict,
    ToolDefinitionScanHit, MAX_PROMPT_BYTES,
};

#[cfg(test)]
#[path = "prompt_injection_tests.rs"]
mod tests;
