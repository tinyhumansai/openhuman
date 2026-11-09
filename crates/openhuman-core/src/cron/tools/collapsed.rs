//! `cron` — the whole scheduler surface as one action-dispatched tool, kept
//! for callers that already name it.
//!
//! The model is shown the six per-operation tools instead (`cron_add`,
//! `cron_list`, `cron_update`, `cron_remove`, `cron_run`, `cron_runs`), the
//! members of the `scheduling` pack: each carries only its own fields, so a
//! call to list or remove a job no longer ships the add schema with it, and
//! the pack decides whether they are on the wire or behind `use_skill`. This
//! tool is [`ToolExposure::Deferred`]: never listed up front, found by
//! `tool_search`, and run when called by name, so a saved skill or an
//! embedder's script that calls `cron` keeps working at no cost per call. An
//! embedder's named scope that lists `cron` is widened to the six (see
//! `widen_named_scope`), so it is shown them too.
//!
//! # It delegates; it does not reimplement
//!
//! Each action forwards to the tool that already served it. That is deliberate
//! and not merely convenient: `cron_add` carries schedule parsing, timezone
//! resolution and a `SecurityPolicy` check, and a second copy of any of that
//! would be a place for the two to disagree about what is allowed. The
//! per-operation tools are the advertised surface; this one only routes to
//! them.
//!
//! # Permissions
//!
//! The six members do not share a permission level: `cron_list` is read-only
//! while `cron_add` is `Execute` (it persists a command that will later run on
//! the host). `permission_level_with_args` resolves the real one once the
//! action is known; the argument-free `permission_level` reports the strictest,
//! so a caller that does not pass arguments over-restricts rather than under-.
//! See `tinytools::collapse` for the reasoning.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{
    add::CronAddTool, list::CronListTool, remove::CronRemoveTool, run::CronRunTool,
    runs::CronRunsTool, update::CronUpdateTool,
};
use crate::config::Config;
use crate::security::policy::SecurityPolicy;
use tinytools::collapse::{
    any_external_effect, args_without_action, merge_action_schemas, resolve, strictest_permission,
    unknown_action_message, CollapsedAction,
};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolExposure, ToolResult};

/// The advertised name. A constant so the registration site, the legacy-alias
/// carve-out and the tests cannot disagree.
pub const CRON_TOOL_NAME: &str = "cron";

pub struct CronTool {
    add: CronAddTool,
    list: CronListTool,
    update: CronUpdateTool,
    remove: CronRemoveTool,
    run: CronRunTool,
    runs: CronRunsTool,
}

impl CronTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self {
            add: CronAddTool::new(Arc::clone(&config), Arc::clone(&security)),
            list: CronListTool::new(Arc::clone(&config)),
            update: CronUpdateTool::new(Arc::clone(&config), security),
            remove: CronRemoveTool::new(Arc::clone(&config)),
            run: CronRunTool::new(Arc::clone(&config)),
            runs: CronRunsTool::new(config),
        }
    }

    /// The action table, in the order it is advertised.
    ///
    /// Rebuilt per call rather than stored because `CollapsedAction` borrows
    /// the members; the cost is six pointer copies and it keeps the type free
    /// of a self-referential field.
    fn actions(&self) -> Vec<CollapsedAction<'_>> {
        vec![
            CollapsedAction {
                action: "list",
                tool: &self.list,
            },
            CollapsedAction {
                action: "add",
                tool: &self.add,
            },
            CollapsedAction {
                action: "update",
                tool: &self.update,
            },
            CollapsedAction {
                action: "remove",
                tool: &self.remove,
            },
            CollapsedAction {
                action: "run",
                tool: &self.run,
            },
            CollapsedAction {
                action: "runs",
                tool: &self.runs,
            },
        ]
    }
}

#[async_trait]
impl Tool for CronTool {
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn name(&self) -> &str {
        CRON_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Manage scheduled jobs. `action`: `list` (all jobs), `add` (create a \
         shell or agent job on a cron/at/every schedule), `update` (patch one), \
         `remove`, `run` (force-run now), `runs` (recent run history). \
         Schedules use the device-local timezone unless `tz` is set; the \
         scheduler polls on an interval and does not catch up missed runs. \
         For agent jobs, a job created inside a conversation sends each run's \
         reply back to that conversation automatically; do not set `delivery` \
         or copy a reply target for that. Only set `delivery` to override it \
         (`proactive` for an in-app notification, `none` for silent). A job \
         created with no conversation defaults to `proactive`. Reminders and \
         recurring messages always deliver; a conditional check must put its \
         condition and \"otherwise reply NO_REPLY\" in its `prompt`, and only \
         such a job can skip a run."
    }

    fn parameters_schema(&self) -> Value {
        merge_action_schemas(&self.actions())
    }

    fn permission_level(&self) -> PermissionLevel {
        strictest_permission(&self.actions())
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        // The honest answer, once the action is known. Falls back to the
        // strictest when the action is missing or unrecognised — such a call is
        // about to be rejected anyway, and answering `None` for it would let an
        // unparseable call past a gate that a parseable one would not clear.
        let actions = self.actions();
        args.get("action")
            .and_then(Value::as_str)
            .and_then(|action| resolve(&actions, action))
            .map(|entry| entry.tool.permission_level_with_args(args))
            .unwrap_or_else(|| strictest_permission(&actions))
    }

    fn external_effect(&self) -> bool {
        any_external_effect(&self.actions())
    }

    fn external_effect_with_args(&self, args: &Value) -> bool {
        let actions = self.actions();
        args.get("action")
            .and_then(Value::as_str)
            .and_then(|action| resolve(&actions, action))
            .map(|entry| entry.tool.external_effect_with_args(args))
            .unwrap_or(true)
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, options, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let actions = self.actions();
        let requested = args.get("action").and_then(Value::as_str);
        let Some(entry) = requested.and_then(|action| resolve(&actions, action)) else {
            return Ok(ToolResult::error(unknown_action_message(
                &actions, requested,
            )));
        };
        tracing::debug!(action = %entry.action, "[tool][cron] dispatch");
        // The selected member owns its context-aware dispatch and option
        // handling. Forward the original call context unchanged, including
        // for members that bind a core or conversation origin.
        entry
            .tool
            .execute_with_context(args_without_action(&args), options, context)
            .await
    }
}

#[cfg(test)]
#[path = "collapsed_tests.rs"]
mod tests;
