//! Prompt injection detection and enforcement.
//!
//! This module centralizes prompt-injection checks so user-provided prompts
//! can be screened before any model or tool execution path.

mod detector;

pub use detector::{
    enforce_prompt_input, scan_tool_definition, PromptEnforcementAction, PromptEnforcementContext,
    PromptEnforcementDecision, PromptInjectionReason, PromptInjectionVerdict,
    ToolDefinitionScanHit,
};

#[cfg(test)]
mod tests;
