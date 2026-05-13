//! Tool-scoped memory layer for durable learnings and high-priority rules.
//!
//! Implements the dedicated memory namespace requested in
//! [issue #1400](https://github.com/tinyhumansai/openhuman/issues/1400):
//! a first-class storage and retrieval surface for **actionable**
//! tool-specific guidance, distinct from the
//! [`tool_effectiveness`](crate::openhuman::learning::tool_tracker)
//! statistics namespace and from the generic `global` / `skill-*`
//! namespaces.
//!
//! ## Namespace convention
//!
//! Each tool gets its own namespace `tool-{tool_name}`. The prefix is
//! distinct from `global`, `skill-{id}`, `tool_effectiveness`, and the
//! learning namespaces so list/clear operations can reason about it
//! without ambiguity. Build the namespace string via
//! [`types::tool_memory_namespace`] — never hard-code the format.
//!
//! ## Components
//!
//! - [`types`] — [`ToolMemoryRule`], [`ToolMemoryPriority`],
//!   [`ToolMemorySource`].
//! - [`store`] — [`ToolMemoryStore`], the put/list/delete/prompt API
//!   built on top of an `Arc<dyn Memory>`.
//! - [`capture`] — [`ToolMemoryCaptureHook`], the post-turn
//!   [`PostTurnHook`] that records user edicts and repeated tool
//!   failures.
//! - [`prompt`] — [`ToolMemoryRulesSection`], the prompt section that
//!   pins Critical / High rules into the system prompt so they survive
//!   mid-session compression.
//!
//! [`PostTurnHook`]: crate::openhuman::agent::hooks::PostTurnHook

pub mod capture;
pub mod prompt;
pub mod store;
#[cfg(test)]
pub mod test_helpers;
pub mod types;

pub use capture::ToolMemoryCaptureHook;
pub use prompt::{render_tool_memory_rules, ToolMemoryRulesSection, TOOL_MEMORY_HEADING};
pub use store::{ToolMemoryStore, TOOL_MEMORY_PROMPT_CAP};
pub use types::{tool_memory_namespace, ToolMemoryPriority, ToolMemoryRule, ToolMemorySource};
