# approval

Interactive approval workflow for supervised mode (issue #1339). `ApprovalGate` is async middleware sitting between the agent and any tool whose `Tool::external_effect` returns `true` (Slack post, email send, calendar create, shell, …). It intercepts the call, checks the user's "Always allow" allowlist, persists a pending row in SQLite, publishes an `ApprovalRequested` event so the UI can surface a prompt, parks the tool-call future on a `oneshot`, and resumes when the UI (or a typed chat yes/no) dispatches a decision via the `approval_decide` RPC. Denials and timeouts (10-min TTL) fail closed. The module also redacts PII/chat content out of anything it persists or broadcasts, and records a terminal execution-outcome audit trail after the allowed tool finishes (issue #2135).

## Responsibilities

- Intercept external-effect tool calls and gate them behind explicit user consent.
- Short-circuit to `Allow` when the tool is on the user's `autonomy.auto_approve` allowlist (read live via `security::live_policy`).
- Allow through (never park) when there is no live chat context — background/triage/cron turns carry no `ApprovalChatContext` and are pre-authorized.
- Persist pending requests in SQLite so they survive a core restart; lazily expire stale rows; keep a durable decided/executed audit trail.
- Resolve a parked call on a user decision (`approve_once` / `approve_always_for_tool` / `deny`), TTL timeout, or channel drop — failing closed in every non-approve path.
- Redact arguments (`redact_args`) and build safe action summaries (`summarize_action`) before anything leaves the gate.
- Route a thread's yes/no chat reply back to a parked approval (`pending_for_thread` + `parse_approval_reply`).
- On `approve_always_for_tool`, persist the tool onto `autonomy.auto_approve` (config save + live-policy reload) so it skips prompting next time.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/approval/mod.rs` | Export-focused: module docstring, `pub mod` decls, `pub use` re-exports including the controller-schema pair. |
| `src/openhuman/approval/gate.rs` | `ApprovalGate` — the singleton coordinator. `init_global`/`try_global`, `intercept`/`intercept_audited`, `decide`, `record_execution`, `list_pending`, `list_recent_decisions`, the thread→request routing map, `ApprovalChatContext` task-local, and `parse_approval_reply`. |
| `src/openhuman/approval/store.rs` | SQLite persistence (`pending_approvals` table). `insert_pending`, `decide`, `get_decision`, `record_execution`, `list_pending`, `list_recent_decisions`, `purge_session`, `expire_stale`, plus idempotent column migration for the v1 schema. |
| `src/openhuman/approval/types.rs` | Serde domain types: `PendingApproval`, `ApprovalAuditEntry`, `ApprovalDecision`, `GateOutcome`, `ExecutionOutcome`. |
| `src/openhuman/approval/redact.rs` | `redact_args` (PII/chat-content key scrubbing + home-path stripping) and `summarize_action` (safe-field summary). |
| `src/openhuman/approval/rpc.rs` | Domain RPC entry points returning `RpcOutcome<T>`: `approval_list_pending`, `approval_list_recent_decisions`, `approval_decide`. |
| `src/openhuman/approval/schemas.rs` | Controller schemas + `handle_*` fns wiring the RPC into the registry. |

## Public surface

Re-exported from `mod.rs`:

- Gate: `ApprovalGate`, `ApprovalChatContext`, `APPROVAL_CHAT_CONTEXT` (task-local), `parse_approval_reply`.
- Redaction: `redact_args`, `summarize_action`.
- Types: `PendingApproval`, `ApprovalAuditEntry`, `ApprovalDecision`, `ExecutionOutcome`, `GateOutcome`.
- Controller registry: `all_approval_controller_schemas`, `all_approval_registered_controllers`.

`ApprovalGate::try_global()` returns `None` when no gate is installed; tools/harness branches treat `None` as "no gating".

## RPC / controllers

Namespace `approval` (registered via `all_approval_registered_controllers`, consumed by `src/core/all.rs`):

| Method | Inputs | Output |
| --- | --- | --- |
| `approval.list_pending` | — | `pending: PendingApproval[]` |
| `approval.list_recent_decisions` | `limit?: u64` (1-500, default 50) | `decisions: ApprovalAuditEntry[]` |
| `approval.decide` | `request_id: string`, `decision: string` (`approve_once` / `approve_always_for_tool` / `deny`) | `decided: PendingApproval` |

`list_pending` / `list_recent_decisions` return empty (not an error) when the gate is not installed; `decide` errors when the gate is absent or the `request_id` is unknown/already decided.

## Agent tools

None. This module gates other domains' tools; it owns no tools of its own (no `tools.rs`).

## Events

Published via `publish_global` (domain `approval`, defined in `src/core/event_bus/events.rs`):

- `DomainEvent::ApprovalRequested { request_id, tool_name, action_summary, args_redacted, session_id, thread_id, client_id }` — emitted when a call is parked. Bridged to the `approval_request` web-channel socket event by `ApprovalSurfaceSubscriber` (defined in `src/openhuman/channels/providers/web.rs`).
- `DomainEvent::ApprovalDecided { request_id, tool_name, decision }` — emitted when a decision is applied.

No `bus.rs` in this module — it only publishes; the subscriber lives in the `channels` web provider.

## Persistence

SQLite DB at `{workspace_dir}/approval/approval.db`, table `pending_approvals` (opened per-call via `with_connection`, schema + column migration applied idempotently). Columns: `request_id` (PK), `tool_name`, `action_summary`, `args_redacted` (JSON), `session_id`, `created_at`, `expires_at`, `decided_at`, `decision`, plus the after-action audit columns `executed_at`, `execution_outcome`, `execution_error` (added by `migrate_columns` for v1 DBs). Pending rows survive restart; expired rows are lazily transitioned to a terminal `deny` decision; `record_execution` is write-once (`executed_at IS NULL` guard) and sanitizes/caps error text to 512 chars to keep secrets/PII out of the durable log.

## Dependencies

- `crate::core::event_bus` — `publish_global` + `DomainEvent` to surface approval prompts/decisions.
- `crate::core::all` — `ControllerFuture` / `RegisteredController` for the controller registry.
- `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) — schema definitions.
- `crate::rpc::RpcOutcome` — RPC return contract.
- `crate::openhuman::config::Config` — workspace dir (DB path) + the boot-time `autonomy.auto_approve` snapshot; `config::ops::add_auto_approve_tool` to persist "Always allow".
- `crate::openhuman::security` — `live_policy::current()` for the live "Always allow" list and `POLICY_DENIED_MARKER` for deny reasons.
- `crate::openhuman::memory_store::safety::sanitize_text` — scrub secrets out of stored execution-error strings.

## Used by

- `src/core/jsonrpc.rs` — installs the global gate (`ApprovalGate::init_global`) at startup; wires the approval RPCs.
- `src/core/all.rs` — registers the controller schemas.
- `src/openhuman/agent/harness/tool_loop.rs` (+ subagent_runner) — routes external-effect tool calls through the gate before `execute()`.
- `src/openhuman/channels/providers/web.rs` — sets `APPROVAL_CHAT_CONTEXT`, hosts `ApprovalSurfaceSubscriber`, and routes typed yes/no replies to `approval_decide`.
- `src/openhuman/channels/proactive.rs`, `src/openhuman/agent/triage/escalation.rs`, `src/openhuman/tools/impl/system/install_tool.rs`, `src/openhuman/wallet/execution.rs` — interact with the gate / approval types.

## Notes / gotchas

- **Interactive only.** With no `ApprovalChatContext` task-local in scope, `intercept` returns `Allow` immediately (no row, no event) so autonomous turns don't stall on a prompt nobody can answer.
- **Fail-closed everywhere.** Persist failure, channel drop, and TTL timeout all return `Deny` (with a `POLICY_DENIED_MARKER`-prefixed reason). The TTL path re-reads the persisted decision to honor an approve that committed in the timeout race (PR #2367).
- **Waiter registered before persist** so a fast `approval_decide` can't mark a request approved while no waiter exists (PR #2149).
- **Orphan rows are intentionally preserved** across launches (issue #1339); deciding one is a DB-only audit update — no side effect can fire across processes, so the security invariant holds.
- **`approve_always_for_tool` persistence is the RPC handler's job**, not the gate's — `gate.decide` only resolves the parked future and emits the audit event; `rpc::approval_decide` appends to `autonomy.auto_approve` + reloads the live policy (best-effort; failure degrades to prompting again).
- `OPENHUMAN_APPROVAL_GATE=0`/`false` skips installing the gate (handled in `src/core/jsonrpc.rs`), in which case `Prompt`-class calls run unprompted.
- A prior list-based `ApprovalManager` was removed; the gate is now the sole control reading the `autonomy.auto_approve` allowlist.
