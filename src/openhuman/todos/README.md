# todos

Per-thread todo list (a.k.a. the **agent task board**): CRUD operations plus a markdown renderer over a thread's task cards. Backed by `crate::openhuman::agent::task_board` for persistence, so agent-side edits via the `todo` tool and user-side edits via the `openhuman.todos_*` RPCs share one source of truth. Each operation loads the current cards for a thread (or a process-global scratch store when there's no thread context), applies a mutation, persists, and returns a `TodosSnapshot` containing both the updated cards and a markdown rendering for the chat UI / agent transcript.

## Responsibilities

- CRUD over a thread's task cards: `list`, `add`, `edit`, `update_status`, `decide_plan`, `remove`, `replace`, `clear`.
- Resolve the working set location: a per-thread `TaskBoardStore` file, or the in-memory process-global scratch store as a fallback when no thread id is available.
- Render a card list as GitHub-flavored markdown (`- [ ]` / `- [x]` / `[~]` in-progress / `[!]` blocked / `[?]` awaiting-approval / `[-]` rejected, plus indented objective/agent/tools/plan/acceptance-criteria/evidence/notes/blocker lines).
- Parse stable string status aliases (`pending`→`Todo`, `approved`→`Ready`, etc.) and approval modes (`required` / `not_required`).
- Enforce the invariant that at most one card is `in_progress` at a time.
- Plan-approval transitions via `decide_plan` (approve → `Ready`/runnable, reject → `Rejected`), guarded so only `AwaitingApproval` cards can be decided.
- Emit `AgentProgress::TaskBoardUpdated` to the forked agent's progress channel after a thread-scoped mutation.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/todos/mod.rs` | Export-focused: declares `ops`/`schemas`/`store`, re-exports the controller-schema pair and the scratch store. |
| `src/openhuman/todos/ops.rs` | Core CRUD business logic. Defines `TodosSnapshot`, `CardPatch`, `BoardLocation`; load/save/markdown helpers; `add`/`edit`/`update_status`/`decide_plan`/`remove`/`replace`/`clear`/`list`; `parse_status`; single-in-progress enforcement; progress emission. Inline test suite. |
| `src/openhuman/todos/schemas.rs` | JSON-RPC controller surface: `all_controller_schemas` / `all_registered_controllers`, per-function `ControllerSchema`s, `handle_*` async handlers, param structs, approval-mode parsing, and `thread_location` resolution. |
| `src/openhuman/todos/store.rs` | `ScratchTodoStore` + `global_scratch_store()` — the process-global in-memory fallback list. |

## Public surface

From `mod.rs` re-exports:

- `all_todos_controller_schemas` / `all_todos_registered_controllers` (aliases of `schemas::all_controller_schemas` / `all_registered_controllers`).
- `ScratchTodoStore`, `global_scratch_store` (from `store`).

Heavily used by callers directly via `todos::ops`: `BoardLocation` (`Thread { workspace_dir, thread_id }` | `Scratch`), `CardPatch`, `TodosSnapshot`, `parse_status`, and the CRUD fns `add`/`edit`/`update_status`/`decide_plan`/`remove`/`replace`/`clear`/`list`/`render_markdown`.

## RPC / controllers

Namespace `todos` (wire methods `openhuman.todos_*`), wired into the registry via `src/core/all.rs`:

| Function | Description |
| --- | --- |
| `list` | Return the thread's cards + markdown. |
| `add` | Append a new card (`content` required; optional status/objective/plan/assignedAgent/allowedTools/approvalMode/acceptanceCriteria/evidence/notes/blocker). |
| `edit` | Edit a card by `id`; omitted fields untouched; `approvalMode: null` clears the mode. |
| `update_status` | Change only the status. |
| `decide_plan` | Approve/reject a card `awaiting_approval` (approve → `ready`, reject → `rejected`). |
| `remove` | Delete a card by `id`. |
| `replace` | Wholesale-replace the list (`cards` JSON array; empty ids are server-generated). |
| `clear` | Empty the list. |

All take a required `thread_id` (same id as `threads.task_board_*`) and return a `snapshot` JSON object (`threadId`, `cards`, `markdown`). `thread_location` loads `Config::load_or_init()` to resolve `workspace_dir`.

## Agent tools

This module owns no `tools.rs`. The agent-facing `todo` tool lives in `src/openhuman/agent/tools/todo.rs` and delegates into `todos::ops`, sharing the same persistence/rendering. It uses `BoardLocation::Scratch` when no thread context is present, else `BoardLocation::Thread`.

## Events

No `bus.rs`. Instead, thread-scoped mutations emit `AgentProgress::TaskBoardUpdated { board }` on the forked agent's progress channel (`agent::harness::fork_context::current_parent().on_progress`). This is an in-process agent-progress channel send, not a `DomainEvent` publish.

## Persistence

- **Per-thread (authoritative):** `TaskBoardStore` writes to `<workspace>/agent_task_boards/<hex(thread_id)>.json` (atomic file rename via `TaskBoardStore::put`; cards normalised with `normalise_board`).
- **Scratch (fallback):** `ScratchTodoStore` — a process-global `Arc<Mutex<Vec<TaskBoardCard>>>` (`global_scratch_store()`), in-memory only, used when no thread id is available. The same `Arc` is returned across calls so tool re-registration keeps state.

## Dependencies

- `crate::openhuman::agent::task_board` — `TaskBoard`, `TaskBoardCard`, `TaskBoardStore`, `TaskCardStatus`, `TaskApprovalMode`, `normalise_board`. The actual card model and per-thread persistence.
- `crate::openhuman::agent::progress::AgentProgress` — variant emitted on board changes.
- `crate::openhuman::agent::harness::fork_context` — `current_parent()` to reach the parent agent's progress sender.
- `crate::openhuman::config::Config` — `load_or_init()` resolves the `workspace_dir` for thread-scoped boards (in `schemas.rs`).
- `crate::core::all` — `ControllerFuture`, `RegisteredController` for the RPC registry.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller schema types.

## Used by

- `src/core/all.rs` — registers the controllers/schemas into the RPC registry.
- `src/openhuman/agent/tools/todo.rs` — the agent `todo` tool delegates to `todos::ops`.
- `src/openhuman/agent/task_dispatcher.rs` — dispatcher reads/transitions cards.
- `src/openhuman/agent/triage/{escalation.rs, envelope.rs}` — triage flows.
- `src/openhuman/task_sources/route.rs` — task-source ingestion (uses `CardPatch.source_metadata`).

## Notes / gotchas

- **Scratch CRUD serialization:** `ScratchTodoStore::snapshot`/`replace` each take the inner lock independently; `ops.rs` wraps scratch load→mutate→save in a coarser process-global `scratch_serial_lock` so the pair runs in one critical section. Thread ops rely on `TaskBoardStore::put`'s file-rename atomicity instead.
- **Single in-progress invariant** is enforced on `add`/`edit`/`replace` (`enforce_single_in_progress`); violating it returns an error rather than silently fixing it.
- **`approval_mode` is doubly-optional** (`Option<Option<TaskApprovalMode>>`): `None` = leave untouched, `Some(None)` = clear, `Some(Some(_))` = set. The `edit` RPC distinguishes "absent" from explicit `null` via `approval_mode_patch_from_params` reading the raw params map before deserialization.
- **`source_metadata`** is only settable via `ops` directly (e.g. task-source route); the `add`/`edit` RPC handlers always pass `source_metadata: None`, and an edit with `None` preserves existing metadata.
- `decide_plan` errors on a non-`AwaitingApproval` card so a stale/duplicate decision can't resurrect a card that already moved on.
- `#[cfg(test)] scratch_test_lock()` is `pub(crate)` so the agent `todo` tool tests can serialize shared-scratch access under cargo's parallel runner.
