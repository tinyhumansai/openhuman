# approval

Interactive approval workflow for supervised mode (issue #1339). `ApprovalGate` is async middleware sitting between the agent and any tool whose `Tool::external_effect` returns `true` (Slack post, email send, calendar create, shell, …). It intercepts the call, checks the user's "Always allow" allowlist, persists a pending row in SQLite, publishes an `ApprovalRequested` event so the UI can surface a prompt, parks the tool-call future on a `oneshot`, and resumes when the UI (or a typed chat yes/no) dispatches a decision via the `approval_decide` RPC. Denials and timeouts (10-min TTL) fail closed. The module also redacts PII/chat content out of anything it persists or broadcasts, and records a terminal execution-outcome audit trail after the allowed tool finishes (issue #2135).

## Responsibilities

- Intercept external-effect tool calls and gate them behind explicit user consent.
- Short-circuit to `Allow` when the tool is on the user's `autonomy.auto_approve` allowlist (read live via `security::live_policy`).
- Cron agent turns may use read-only tools and the job's configured delivery. Their external-effect calls are denied, including schedule mutation. Other automation follows its origin-specific gate policy.
- Persist pending requests in SQLite so they survive a core restart; lazily expire stale rows; keep a durable decided/executed audit trail.
- Resolve a parked call on a user decision (`approve_once` / `approve_always_for_tool` / `deny`), TTL timeout, or channel drop, failing closed in every non-approve path.
- Redact arguments (`redact_args`) and build safe action summaries (`summarize_action`) before anything leaves the gate.
- Route a thread's yes/no chat reply back to a parked approval (`pending_for_thread` + `parse_approval_reply`).
- On `approve_always_for_tool`, persist the tool onto `autonomy.auto_approve` (config save + live-policy reload) so it skips prompting next time.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/security/approval/mod.rs` | Export-focused: module docstring, `pub mod` decls, `pub use` re-exports including the controller-schema pair. |
| `crates/openhuman-core/src/security/approval/gate.rs` | `ApprovalGate` struct + `DecideMiss`, `DEFAULT_APPROVAL_TTL` (10 minutes) and the shorter `COPILOT_APPROVAL_TTL` and `SUBAGENT_APPROVAL_TTL` (3 minutes, any park inside a sub-agent run), `APPROVAL_UNANSWERED_PHRASE` / `is_unanswered_approval_reason` (TTL expiry vs refusal), `is_detached_request` (a park routed to its chat thread from an async sub-agent, marked `detached` on the `approval_request` web event so the client keeps it past the parent turn), the `ApprovalChatContext` / `FlowRunContext` task-locals, `parse_approval_reply`, and the `ApprovalGateBootState` record. |
| `crates/openhuman-core/src/security/approval/gate_setup.rs` (`include!`d by [`gate.rs`](./gate.rs), as are the next two) | `ApprovalGate::init_global`/`try_global` (process-global install, re-install-safe) and the private constructor. |
| `crates/openhuman-core/src/security/approval/gate_intercept.rs` | `intercept`/`intercept_audited`/`intercept_audited_bounded`: the origin check, allowlist short-circuit, persist-and-park flow, and cancellation-safe bounded park used by the Flow Canvas copilot live-run path. |
| `crates/openhuman-core/src/security/approval/gate_state.rs` | `decide` (resolves the parked future, emits `ApprovalDecided`), `classify_decide_miss`, `record_execution` (best-effort terminal audit row), `list_pending`/`list_recent_decisions`, the flow-trust helpers, and the thread→request routing lookups. |
| `crates/openhuman-core/src/security/approval/store.rs` | SQLite persistence (`pending_approvals` table). `insert_pending`, `decide`, `get_decision`, `record_execution`, `list_pending`, `list_recent_decisions`, `purge_session`, `expire_stale`, plus idempotent column migration for the v1 schema. |
| `crates/openhuman-core/src/security/approval/types.rs` | Serde domain types: `PendingApproval`, `ApprovalAuditEntry`, `ApprovalDecision`, `GateOutcome`, `ExecutionOutcome`. |
| `crates/openhuman-core/src/security/approval/redact.rs` | `redact_args` (PII/chat-content key scrubbing + home-path stripping) and `summarize_action` (safe-field summary). |
| `crates/openhuman-core/src/security/approval/rpc.rs` | Domain RPC entry points returning `Outcome<T>`: `approval_get_gate_state`, `approval_list_pending`, `approval_list_recent_decisions`, `approval_decide`, `approval_preauthorize_flow`. |
| `crates/openhuman-core/src/security/approval/schemas.rs` | Controller schemas + `handle_*` fns wiring the RPC into the registry. |

## Public surface

Re-exported from [`mod.rs`](./mod.rs):

- Gate: `ApprovalGate`, `ApprovalChatContext`, `FlowRunContext`, `ApprovalTurnContext` (the explicit per-turn chat / copilot / flow-run context every gate check takes), `parse_approval_reply`.
- Redaction: `redact_args`, `summarize_action`.
- Types: `PendingApproval`, `ApprovalAuditEntry`, `ApprovalDecision`, `ApprovalSourceContext`, `ExecutionOutcome`, `GateOutcome`.
- Controller registry: `all_approval_controller_schemas`, `all_approval_registered_controllers`.

`ApprovalGate::try_global()` returns `None` when no gate is installed; tools/harness branches treat `None` as "no gating".

## RPC / controllers

Namespace `approval` (registered via `all_approval_registered_controllers`, consumed by `crates/openhuman-core/src/core/all.rs`):

| Method | Inputs | Output |
| --- | --- | --- |
| `approval.list_pending` | none | `pending: PendingApproval[]` |
| `approval.list_recent_decisions` | `limit?: u64` (1-500, default 50) | `decisions: ApprovalAuditEntry[]` |
| `approval.get_gate_state` | none | `state: ApprovalGateBootState` (installed / disabled-by-env / override-ignored / host tag) |
| `approval.decide` | `request_id: string`, `decision: string` (`approve_once` / `approve_always_for_tool` / `approve_always_for_flow` / `deny`) | `decided: PendingApproval` |
| `approval.preauthorize_flow` | `flow_id: string`, `tool_names: string[]` | `result: FlowPreauthorizationResult` (idempotent flow-scoped trust grants; succeeds with `gate_installed=false` when the gate is off) |

`list_pending` / `list_recent_decisions` return empty (not an error) when the gate is not installed; `decide` errors when the gate is absent or the `request_id` is unknown/already decided.

## Agent tools

None. This module gates other domains' tools; it owns no tools of its own (no `tools.rs`).

## Events

Published via `crate::core::bus::BUS.publish` with variants from `crate::core::events::DomainEvent` (`crates/openhuman-core/src/core/events.rs`):

- `DomainEvent::ApprovalRequested { request_id, tool_name, action_summary, args_redacted, session_id, thread_id, client_id }`: emitted ([`gate_intercept.rs`](./gate_intercept.rs)) when a call is parked. Bridged to the `approval_request` web-channel socket event by `ApprovalSurfaceSubscriber` (defined in `crates/openhuman-core/src/web_chat/`).
- `DomainEvent::ApprovalDecided { request_id, tool_name, decision }`: emitted ([`gate_state.rs`](./gate_state.rs)) when a decision is applied.
- `DomainEvent::FlowApprovalRequested { request_id, flow_id, run_id, tool_name, summary }`: emitted (`gate_intercept.rs`) alongside `ApprovalRequested` when the parked call has a `Workflow` origin. It carries no thread/client id, so `ApprovalSurfaceSubscriber` drops it; `openhuman_rpc::server::socketio` broadcasts it as `flow_approval_request` for the Workflows UI.

No `bus.rs` in this module. It only publishes; the subscriber (`ApprovalSurfaceSubscriber`) lives in `crates/openhuman-core/src/web_chat/event_bus.rs`.

## Persistence

SQLite DB at `{workspace_dir}/approval/approval.db`, table `pending_approvals` (opened per-call via `with_connection`, schema + column migration applied idempotently). Columns: `request_id` (PK), `tool_name`, `action_summary`, `args_redacted` (JSON), `session_id`, `created_at`, `expires_at`, `decided_at`, `decision`, plus the after-action audit columns `executed_at`, `execution_outcome`, `execution_error` (added by `migrate_columns` for v1 DBs). Pending rows survive restart; expired rows are lazily transitioned to a terminal `deny` decision; `record_execution` is write-once (`executed_at IS NULL` guard) and sanitizes/caps error text to 512 chars to keep secrets/PII out of the durable log.

### On a storage backend

When the host configured a storage backend (`OPENHUMAN_STORAGE_URL` /
`[storage] url`, see `crate::storage`), every `store` function uses
`store_documents.rs` instead of `approval.db`: the same operations on the
`tinystoragedrivers` document port, under the current call's storage scope
(the acting agent; `local` on a single-user host; refused in SaaS mode with
no acting agent). Collections `approvals` (one document per `request_id`)
and `approval_flow_trust` (one per `(flow_id, tool_name)`). The SQL guards
become preconditions: insert-only for new requests, compare-and-swap for
decide, expiry and `record_execution`, so two processes on one database
decide a request at most once. `args_redacted` and `source_context` are
stored as JSON strings. With no backend configured (the desktop default)
`approval.db` is used exactly as above.

## Dependencies

- `crate::core::bus::BUS` + `crate::core::events::DomainEvent` to surface approval prompts/decisions.
- `crate::core::all`: `ControllerFuture` / `RegisteredController` for the controller registry.
- `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`): schema definitions.
- `crate::core::Outcome`: RPC return contract.
- `crate::config::Config`: workspace dir (DB path) plus the boot-time `autonomy.auto_approve` snapshot; `config::ops::add_auto_approve_tool` persists "Always allow".
- `crate::security`: `live_policy::current()` for the live "Always allow" list and `POLICY_DENIED_MARKER` for deny reasons.
- `tinymemory_core::store::safety::sanitize_text`: scrub secrets out of stored execution-error strings.

## Used by

- `crates/openhuman-core/src/core/runtime/bootstrap.rs`: installs the global gate (`ApprovalGate::init_global`) at startup; `core/all.rs` registers the approval RPC controllers.
- `crates/openhuman-core/src/core/all.rs`: registers the controller schemas.
- `crates/openhuman-core/src/agent/tinyagents/middleware.rs` (`ApprovalSecurityMiddleware`, a `wrap_tool` middleware on every turn path): routes external-effect tool calls through the gate before `execute()` and records the terminal audit row.
- `crates/openhuman-core/src/web_chat/`: hands each turn its chat `ApprovalTurnContext`, hosts `ApprovalSurfaceSubscriber`, and routes typed yes/no replies to `approval_decide`.
- `crates/openhuman-core/src/channels/proactive.rs`, `crates/openhuman-core/src/agent/triage/escalation.rs`, `crates/openhuman-core/src/tools/impl/system/install_tool.rs`, `crates/openhuman-core/src/web3/wallet/execution.rs`: interact with the gate / approval types.

## Notes / gotchas

- **Fail-closed 10-minute TTL is an invariant, not a tunable.** [`gate.rs`](./gate.rs)'s
  `DEFAULT_APPROVAL_TTL` (`Duration::from_secs(60 * 10)`) matches the
  default `expires_at` written into the persisted row; a parked call that
  times out resolves to `Deny`. Do not weaken this default or the fail-closed
  timeout behavior to make a feature work.
- **Interactive only.** With no `ApprovalChatContext` task-local in scope, `intercept` returns `Allow` immediately (no row, no event) so autonomous turns don't stall on a prompt nobody can answer.
- **Fail-closed everywhere.** Persist failure, channel drop, and TTL timeout all return `Deny` (with a `POLICY_DENIED_MARKER`-prefixed reason). The TTL path re-reads the persisted decision to honor an approve that committed in the timeout race (PR #2367).
- **Waiter registered before persist** so a fast `approval_decide` can't mark a request approved while no waiter exists (PR #2149).
- **Orphan rows are intentionally preserved** across launches (issue #1339); deciding one is a DB-only audit update, and no side effect can fire across processes, so the security invariant holds.
- **`approve_always_for_tool` persistence is the RPC handler's job**, not the gate's: `gate.decide` only resolves the parked future and emits the audit event; `rpc::approval_decide` appends to `autonomy.auto_approve` and reloads the live policy (best-effort; failure degrades to prompting again).
- `OPENHUMAN_APPROVAL_GATE=0`/`false` skips installing the gate for CLI, Docker, and library hosts only (`approval_gate_boot_decision` in `crates/openhuman-core/src/core/types.rs`, applied in `core/runtime/bootstrap.rs`); the Tauri shell always installs it and ignores the override. Where honored, `Prompt`-class calls run unprompted.
- **A disabled autonomy policy parks nothing.** With `[autonomy] enabled = false` (the shipped default, see `security/README.md`) `SecurityPolicy::gate_decision` answers `Allow` for every class, so the acting tools' `external_effect_with_args` (which asks for `GateDecision::Prompt`) is `false` and the gate is never reached. The gate is still installed and still works the moment the policy is turned on; nothing here is special-cased on the flag.
- A prior list-based `ApprovalManager` was removed; the gate is now the sole control reading the `autonomy.auto_approve` allowlist.

## Tests

- [`gate_tests.rs`](./gate_tests.rs), [`store_tests.rs`](./store_tests.rs), [`redact_tests.rs`](./redact_tests.rs), [`schemas_tests.rs`](./schemas_tests.rs), [`types_tests.rs`](./types_tests.rs).

## Further reading

- [Parent module (`security`)](../README.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
