//! `todo` — the session's todo list, the way Claude Code and Codex have it.
//!
//! The tool itself is TinyAgents' `todos::TodoTool` (schema, argument
//! validation, the whole-list write, markdown). This file is only the host
//! adapter: it selects the thread-scoped list for a turn and registers the
//! harness dispatch. Bad arguments must become a tool error, never a fatal
//! harness error.
//!
//! **Scope key.** The list is keyed by the chat **thread id**
//! (`ToolRunContext::thread_id`) rather than `ParentExecutionContext::session_id`
//! — for the web channel, `session_id` is the `{client_id,thread_id}` JSON
//! blob (`fork_context.rs`), which changes with the client and is not what
//! `threads.todos_get` or the `thread_todos_changed` socket event key on. A
//! thread id is stable across reconnects and matches every other thread-scoped
//! surface (goals, turn state). Older lists written under the legacy
//! `session_id` key before this change are found via a one-time fallback read
//! (see [`current_scope`] / [`legacy_session_key`]) so an in-flight list isn't
//! dropped by the rekey.

use crate::agent::subagent_host::ParentExecutionContext;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinyagents_harness_tools::todos::{self as harness_todos, store as todos_store};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};

pub struct TodoTool {
    inner: harness_todos::TodoTool,
    workspace_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoScope {
    Session { id: String },
    Scratch,
}

impl TodoScope {
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Session { id } => Some(id),
            Self::Scratch => None,
        }
    }

    fn key(&self) -> &str {
        self.session_id().unwrap_or("_scratch_")
    }
}

pub(crate) struct TodoToolDispatch {
    tool: Arc<dyn Tool>,
}

impl TodoToolDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext> for TodoToolDispatch {
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }

    async fn execute(
        &self,
        _state: &(),
        call_id: tinyagents_harness::CallId,
        arguments: serde_json::Value,
        _options: ToolCallOptions,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let context = ToolExecutionContext::from_run_context(parent, call_id);
        let workspace_dir = match parent.data.parent.as_ref() {
            Some(parent_ctx) => parent_ctx.workspace_dir.clone(),
            None => crate::config::ops::load_current_or_init(&parent.data.dispatch_ctx())
                .await
                .map(|c| c.workspace_dir)
                .map_err(|e| anyhow::anyhow!("[tool][todo] load config: {e}"))?,
        };
        let writes = writes_plan(&arguments);
        let scope = current_scope(parent.data.parent.as_ref(), Some(&context));
        let result = TodoTool::new(workspace_dir)
            .execute_with_parent_context(arguments, parent.data.parent.clone(), Some(&context))
            .await?;
        if let (Some(id), Some(todos)) = (scope.session_id(), changed_todos(writes, &result)) {
            crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ThreadTodosChanged {
                thread_id: id.to_string(),
                todos,
            });
        }
        Ok(result)
    }
}

/// Whether a `todo` call asks to change the plan: the whole list (`todos`), a
/// `complete` or a `cancel`, in any of their shapes (one step, a list, a list
/// sent as a string, or `complete` and `cancel` together). A read (`{}`) or a
/// call that carries only notes (`facts`, `blockers`, `approach`, `retract`)
/// leaves the plan as it was.
fn writes_plan(arguments: &serde_json::Value) -> bool {
    ["todos", "complete", "cancel"]
        .iter()
        .any(|key| arguments.get(*key).is_some())
}

/// The list to send the frontend's todo drawer after a call, or `None` when
/// the plan did not change: the call did not ask to (`writes` from
/// [`writes_plan`]), or it was refused (an out-of-order tick, a bad step), in
/// which case the list is as it was. A re-sent unchanged list would only be a
/// redundant socket event.
fn changed_todos(writes: bool, result: &ToolResult) -> Option<serde_json::Value> {
    if !writes || result.is_error {
        return None;
    }
    let todos = serde_json::from_str::<serde_json::Value>(&result.output())
        .ok()
        .and_then(|payload| payload.get("todos").cloned());
    if todos.is_none() {
        tracing::debug!(
            "[tool][todo] write succeeded but result had no `todos` field — skipping ThreadTodosChanged"
        );
    }
    todos
}

impl TodoTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            inner: harness_todos::TodoTool::new(
                crate::storage::sessions::stores_or_ephemeral(&workspace_dir).kv,
            ),
            workspace_dir,
        }
    }
}

/// Supplies the selected session key to TinyAgents' tool implementation.
struct ScopedKey<'a>(&'a str);

impl ToolRunContext for ScopedKey<'_> {
    fn thread_id(&self) -> Option<&str> {
        Some(self.0)
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.parameters_schema()
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_parent_context(args, None, tool_context)
            .await
    }
}

impl TodoTool {
    async fn execute_with_parent_context(
        &self,
        args: serde_json::Value,
        parent: Option<ParentExecutionContext>,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let scope = current_scope(parent.as_ref(), tool_context);
        // One-time fallback: a list written under the pre-rekey
        // `session_id` key (the web channel's `{client_id,thread_id}` JSON
        // blob) is otherwise invisible once `current_scope` starts keying by
        // thread id. If the new key has no list yet and the legacy key does,
        // migrate it forward so an in-flight list isn't dropped by the rekey.
        if let Some(legacy_key) = legacy_session_key(parent.as_ref(), &scope) {
            self.migrate_legacy_list_if_absent(&scope, &legacy_key)
                .await;
        }
        tracing::debug!(session_id = ?scope.session_id(), "[tool][todo] dispatch");
        let key = ScopedKey(scope.key());
        self.inner
            .execute_with_context(args, ToolCallOptions::default(), Some(&key))
            .await
    }

    /// If `scope` has no stored list and `legacy_key` has a non-empty one,
    /// copy it forward under `scope`'s key so the rekey is transparent to an
    /// in-flight session. A persisted empty list is authoritative too: it
    /// records that the user cleared the migrated list and must not be treated
    /// as an absent target. Best-effort: store errors are logged and swallowed;
    /// the legacy value remains untouched unless the copy succeeds.
    async fn migrate_legacy_list_if_absent(&self, scope: &TodoScope, legacy_key: &str) {
        let store = crate::storage::sessions::stores_or_ephemeral(&self.workspace_dir).kv;
        match todos_store::get(&store, scope.key()).await {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(error = %e, "[tool][todo] legacy-migration: current list read failed");
                return;
            }
        }
        match todos_store::list(&store, legacy_key).await {
            Ok(legacy) if !legacy.items.is_empty() => {
                tracing::info!(
                    legacy_key,
                    thread_key = scope.key(),
                    items = legacy.items.len(),
                    "[tool][todo] migrating legacy session-keyed list to thread-keyed list"
                );
                if let Err(e) = todos_store::replace(&store, scope.key(), legacy.items).await {
                    tracing::debug!(error = %e, "[tool][todo] legacy-migration: write failed");
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(error = %e, "[tool][todo] legacy-migration: legacy list read failed");
            }
        }
    }
}

/// The scope this call resolves to: the chat thread id when available,
/// falling back to the legacy `ParentExecutionContext::session_id` (for
/// non-web-chat callers that never carry a `thread_id`), and finally the
/// scratch scope for a bare `Tool::execute` with neither.
fn current_scope(
    parent: Option<&ParentExecutionContext>,
    tool_context: Option<&dyn ToolRunContext>,
) -> TodoScope {
    if let Some(thread_id) = tool_context.and_then(ToolRunContext::thread_id) {
        return TodoScope::Session {
            id: thread_id.to_owned(),
        };
    }
    match parent {
        Some(parent) => TodoScope::Session {
            id: parent.session_id.clone(),
        },
        None => TodoScope::Scratch,
    }
}

/// The pre-rekey `session_id` key to check as a one-time fallback, when it
/// differs from the scope's own (now thread-id-first) key.
fn legacy_session_key(
    parent: Option<&ParentExecutionContext>,
    scope: &TodoScope,
) -> Option<String> {
    let parent = parent?;
    if parent.session_id == scope.key() {
        return None;
    }
    Some(parent.session_id.clone())
}

#[cfg(test)]
#[path = "todo_tests.rs"]
mod tests;
