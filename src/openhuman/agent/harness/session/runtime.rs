//! Public accessors, `run_single` / `run_interactive` CLI helpers, and
//! assorted per-turn static helpers (id-fallback injection, event-error
//! sanitisation, history diffing).
//!
//! These used to live alongside the turn loop in `agent.rs`. Splitting
//! them out keeps `turn.rs` focused on the interaction lifecycle and
//! makes it obvious which methods are cheap getters vs which actually
//! drive the model.

use super::types::{Agent, AgentBuilder};
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::dispatcher::ParsedToolCall;
use crate::openhuman::agent::error::AgentError;
use crate::openhuman::agent::messages::ConversationMessage;
use crate::openhuman::inference::provider::{self, ToolCall};
use crate::openhuman::memory::Memory;
use crate::openhuman::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::openhuman::tools::agent_policy::ToolPolicyEngine;
use crate::openhuman::tools::{Tool, ToolSpec};
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
include!("runtime_impl_01_part_01.rs");
include!("runtime_impl_01_part_02.rs");

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
