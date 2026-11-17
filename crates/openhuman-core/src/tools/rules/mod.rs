//! OpenHuman's composition of tool rules.
//!
//! The vocabulary is [`tinytools::ToolRules`] and the enforcement is the
//! tinyagents loop's tool gate (`RunPolicy::tool_rules`), which applies one
//! rule set to the catalogue, `tool_search` and every call. This module only
//! decides *which* rules a turn carries and in what context:
//!
//! - `[tool_rules]` in `config.toml` — the operator's global layer;
//! - an agent definition's `disallowed_tools` (name globs) and `tool_rules`;
//! - a sub-agent inherits its parent's layers and adds its own, so a child is
//!   never less restricted than the run that spawned it.
//!
//! Layers stack in a [`tinytools::ToolRuleSet`]: every layer must admit a
//! tool, so adding one can only narrow.
//!
//! The context the rules' `when` conditions match is built by
//! [`rule_context`]: the session's `channel` and `agent`. The turn's origin is
//! not used: the host-rendered catalogue is cached per session, so the rules
//! must decide the same way on every turn.

mod ops;

pub use ops::{
    agent_rule_layer, child_rule_policy, glob_list_matches, install_turn_rules, rule_context,
    session_rule_set, turn_rule_policy,
};

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
