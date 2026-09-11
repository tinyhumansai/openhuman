//! The agent-facing tool for the long-term goals list.
//!
//! This is what the background `goals_agent` (and, when allowed, the main
//! agent) uses to read and mutate the goals list over multiple turns.
//!
//! # One tool with an `op`, not four tools (#5316-adjacent)
//!
//! It was four — `goals_list` / `goals_add` / `goals_edit` / `goals_delete` —
//! and they differed only in which [`ops`] function they called. Four schemas
//! is four copies of the same `{"type":"object", …}` preamble charged on every
//! provider call of every turn, for one family the model touches rarely. The
//! shape here follows [`todo`](crate::openhuman::agent::tools::todo): an `op`
//! enum carrying no description of its own, flat optional siblings whose
//! descriptions name the ops that need them, and a `match` on `op`.
//!
//! Permission is **per op**, not flat, which is why
//! [`Tool::permission_level_with_args`] is implemented: `list` is `ReadOnly`
//! and the three mutations are `Write`, and collapsing them to a single flat
//! `Write` would put an approval prompt in front of reading the list. The
//! pattern is
//! `git_operations`'s — resolve the
//! discriminator from the args and answer for that branch, with the arg-less
//! accessor reporting the ceiling rather than guessing low.
//!
//! # It wraps [`goals::ops`], not a store (#5560)
//!
//! Each tool used to call `tinycortex::memory::goals::store` against a
//! `workspace_dir` captured at construction. Both halves of that changed:
//!
//! - The **store** is behind the loaded module now, reached through the goals
//!   family on the guarded driver.
//! - The **validation** — the secret/PII and single-line guards — is host
//!   policy that the family deliberately does not carry, and it lives in
//!   [`goals::doc`](crate::openhuman::memory::goals::doc).
//!
//! Routing through [`goals::ops`](crate::openhuman::memory::goals::ops) is what
//! keeps those two facts in one place. When these tools called the store
//! directly they duplicated the RPC surface's sequence, so the tool path and
//! the `memory_goals.*` path were two implementations of one operation; now
//! they are the same one, and a goal added by the agent is validated,
//! capped and reported exactly as a goal added over RPC.
//!
//! The constructor still takes a `workspace_dir` and it is still what
//! `tools::ops` passes: it is the sandbox identity this tool was built with,
//! and dropping it would change a public signature in the agent's tool
//! registry for no behavioural gain. It is unused for storage — the goals
//! document is workspace-wide on the driver's side too, so the tool and the
//! ambient binding name the same file.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::goals::ops;
use crate::openhuman::memory::guard::MemoryGuard;
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// The guarded driver for this call, checked to serve the goals family.
///
/// Returned as the guard because the family accessor borrows from it. The
/// error text is what the agent reads back as a tool failure, so it names the
/// missing capability rather than an internal path.
async fn goals_guard() -> Result<std::sync::Arc<MemoryGuard>, String> {
    let guard = active_memory_guard().await?;
    if guard.as_goals().is_none() {
        return Err("memory driver does not support the goals family".to_string());
    }
    Ok(guard)
}

/// The op named in `args`, lowercased for the permission and dispatch lookups.
fn requested_op(args: &serde_json::Value) -> Option<&str> {
    args.get("op").and_then(|v| v.as_str())
}

/// `goals` — read and mutate the user's long-term goals list.
pub struct GoalsTool {
    workspace_dir: PathBuf,
}

impl GoalsTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for GoalsTool {
    fn name(&self) -> &str {
        "goals"
    }

    fn description(&self) -> &str {
        "Read and edit the user's long-term goals: durable objectives for \
         working with them, not this thread's task list (that is `todo`). \
         Start with `op: \"list\"` so you address the right ids and avoid \
         duplicates."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["op"],
            "properties": {
                "op": { "type": "string", "enum": ["list", "add", "edit", "delete"] },
                "id": { "type": "string", "description": "Goal id, as returned by `list` (required for edit/delete)." },
                "text": { "type": "string", "description": "One concise sentence (required for add/edit)." }
            }
        })
    }

    /// The ceiling. Nothing here says which op is being asked for, so reporting
    /// `ReadOnly` would admit a write on a channel that refuses one.
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn permission_level_with_args(&self, args: &serde_json::Value) -> PermissionLevel {
        match requested_op(args) {
            Some("list") => PermissionLevel::ReadOnly,
            // Every other op mutates, and an absent or unknown one fails the
            // call anyway — report the ceiling rather than a permissive guess.
            _ => PermissionLevel::Write,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let Some(op) = requested_op(&args) else {
            return Ok(ToolResult::error(
                "Missing 'op' parameter (list, add, edit or delete)",
            ));
        };
        let text = args.get("text").and_then(|v| v.as_str());
        let id = args.get("id").and_then(|v| v.as_str());

        // Validate arguments BEFORE resolving the driver, so a malformed call
        // reports the argument it is missing whatever the memory driver is.
        // The four tools this replaced each did their own validation first, and
        // `missing_arguments_are_reported_as_tool_errors` pins the ordering.
        let action = match op {
            "list" => Action::List,
            "add" => match text {
                Some(text) => Action::Add(text),
                None => return Ok(ToolResult::error("Missing 'text' parameter for op 'add'")),
            },
            "edit" => match (id, text) {
                (Some(id), Some(text)) => Action::Edit(id, text),
                (None, _) => return Ok(ToolResult::error("Missing 'id' parameter for op 'edit'")),
                (_, None) => {
                    return Ok(ToolResult::error("Missing 'text' parameter for op 'edit'"))
                }
            },
            "delete" => match id {
                Some(id) => Action::Delete(id),
                None => return Ok(ToolResult::error("Missing 'id' parameter for op 'delete'")),
            },
            other => {
                return Ok(ToolResult::error(format!(
                    "Unknown op '{other}'. Expected one of: list, add, edit, delete"
                )))
            }
        };

        log::debug!("[memory_goals] tool=goals op={op}");
        let guard = match goals_guard().await {
            Ok(guard) => guard,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let goals = guard.as_goals().expect("checked in goals_guard");

        match action {
            Action::List => match ops::list(goals).await {
                // `render()` is the contract type's own markdown, so the bytes
                // the agent reads are unchanged.
                Ok(outcome) => Ok(ToolResult::success(outcome.value.render())),
                Err(e) => Ok(ToolResult::error(e)),
            },
            Action::Add(text) => match ops::add(goals, text).await {
                Ok(outcome) => Ok(ToolResult::success(format!(
                    "Added goal '{}'.",
                    outcome.value.id
                ))),
                Err(e) => Ok(ToolResult::error(e)),
            },
            Action::Edit(id, text) => match ops::edit(goals, id, text).await {
                Ok(_) => Ok(ToolResult::success(format!("Edited goal '{id}'."))),
                Err(e) => Ok(ToolResult::error(e)),
            },
            Action::Delete(id) => match ops::delete(goals, id).await {
                Ok(_) => Ok(ToolResult::success(format!("Deleted goal '{id}'."))),
                Err(e) => Ok(ToolResult::error(e)),
            },
        }
    }
}

/// One validated op, borrowed from the call's arguments.
///
/// It exists so argument validation can finish before the memory driver is
/// resolved: the driver round trip can fail for reasons that have nothing to do
/// with the call being malformed, and reporting that instead of the missing
/// argument is what sends the agent debugging the wrong thing.
enum Action<'a> {
    List,
    Add(&'a str),
    Edit(&'a str, &'a str),
    Delete(&'a str),
}

#[cfg(test)]
#[path = "goals_tests.rs"]
mod tests;
