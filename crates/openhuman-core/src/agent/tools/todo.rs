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

use crate::agent::harness::fork_context::ParentExecutionContext;
use crate::agent::todos::ops::{self, TodoScope};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tinyagents_graph::todos as graph_todos;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};

pub struct TodoTool {
    inner: graph_todos::TodoTool,
    workspace_dir: PathBuf,
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
        let is_write = ["todos", "complete", "cancel"]
            .iter()
            .any(|key| arguments.get(*key).is_some());
        let scope = current_scope(parent.data.parent.as_ref(), Some(&context));
        let result = TodoTool::new(workspace_dir)
            .execute_with_parent_context(arguments, parent.data.parent.clone(), Some(&context))
            .await?;
        // Only a write (the whole list, a `complete` or a `cancel`) changes
        // anything the frontend's todo drawer needs to hear about; a bare read
        // (`{}`) re-reports the same list and would just be a redundant socket
        // event.
        if is_write && !result.is_error {
            if let Some(id) = scope.session_id() {
                match serde_json::from_str::<serde_json::Value>(&result.output())
                    .ok()
                    .and_then(|payload| payload.get("todos").cloned())
                {
                    Some(todos) => {
                        crate::core::bus::BUS.publish(
                            crate::core::events::DomainEvent::ThreadTodosChanged {
                                thread_id: id.to_string(),
                                todos,
                            },
                        );
                    }
                    None => {
                        tracing::debug!(
                            thread_id = id,
                            "[tool][todo] write succeeded but result had no `todos` field — skipping ThreadTodosChanged"
                        );
                    }
                }
            }
        }
        Ok(result)
    }
}

impl TodoTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            inner: graph_todos::TodoTool::new(ops::store(&workspace_dir)),
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

    /// If `scope`'s list is empty and `legacy_key` has a non-empty one,
    /// copy it forward under `scope`'s key so the rekey is transparent to an
    /// in-flight session. Best-effort: any store error is logged and
    /// swallowed — a failed migration just means the tool starts from an
    /// empty list, same as any other first `todo` call.
    async fn migrate_legacy_list_if_absent(&self, scope: &TodoScope, legacy_key: &str) {
        let current = match ops::list(&self.workspace_dir, scope).await {
            Ok(snapshot) => snapshot,
            Err(e) => {
                tracing::debug!(error = %e, "[tool][todo] legacy-migration: current list read failed");
                return;
            }
        };
        if !current.items.is_empty() {
            return;
        }
        let legacy_scope = TodoScope::Session {
            id: legacy_key.to_string(),
        };
        match ops::list(&self.workspace_dir, &legacy_scope).await {
            Ok(legacy) if !legacy.items.is_empty() => {
                tracing::info!(
                    legacy_key,
                    thread_key = scope.key(),
                    items = legacy.items.len(),
                    "[tool][todo] migrating legacy session-keyed list to thread-keyed list"
                );
                if let Err(e) = ops::replace(&self.workspace_dir, scope, legacy.items).await {
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
