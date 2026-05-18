//! Per-thread todo list (a.k.a. agent task board) — CRUD operations plus
//! a markdown renderer. Backed by [`crate::openhuman::agent::task_board`]
//! for persistence, so agent-side edits via the `todo` tool and user-side
//! edits via `openhuman.todos_*` RPCs share the same source of truth.
//!
//! Design notes:
//! - **Per-thread scoped.** The current agent thread id (or an explicit
//!   `thread_id` from the RPC caller) selects which board to mutate.
//! - **In-memory scratch.** When no thread context is available the
//!   process-global scratch store is used (legacy fallback for tool
//!   invocations outside a chat thread).
//! - **Markdown output.** Both tool results and RPC responses include a
//!   `markdown` string so the chat UI / agent transcript can render the
//!   list directly without re-formatting.

pub mod ops;
pub mod schemas;
pub mod store;

pub use schemas::{
    all_controller_schemas as all_todos_controller_schemas,
    all_registered_controllers as all_todos_registered_controllers,
};
pub use store::{global_scratch_store, ScratchTodoStore};
