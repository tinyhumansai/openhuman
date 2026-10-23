# approval

Interactive approval workflow for supervised mode (issue #1339). `ApprovalGate` is async middleware sitting between the agent and any tool whose `Tool::external_effect` returns `true` (Slack post, email send, calendar create, shell, …). It asks the approval engine whether the call may run, publishes an `ApprovalRequested` event when a person must decide, parks the tool-call future on a `oneshot`, and resumes when the UI (or a typed chat yes/no) dispatches a decision via the `approval_decide` RPC. Denials and timeouts (10-min window) fail closed.

**The engine is the TinySecurity module.** The durable lifecycle of a request (pending, approved, denied, expired, cancelled, reserved, executed), its exact binding to the call, the user's tool and flow grants, the expiry and the audit trail are the module's (`tinysecurity-bus` contract 1.18: `EvaluateApproval2`, `Decide2`, `ConsumeApproval2`, `RecordExecution2`, `ExpireApproval2`, `CancelApproval2`, `RegisterApprovalScope2`, `ReplaceApprovalScope2`). The host keeps what is its own: the verified origin of a call, the user's configuration, the parked future, chat routing and the product events, and it serves the module's callbacks (store, audit sink, prompt, human verification). The old host engine (the SQLite and document-port lifecycle, the lazy expiry, the host audit row) is gone.

## Responsibilities

- Label the call's origin (`origin_class`) and hand it to the engine; what each class is allowed is the module's decision (contract 1.22): a cron turn's external effect is denied, an unlabelled call is denied unless the user's Always-allow list names that exact tool, and internal background jobs, pre-declared workflow actions and the CLI are audited automatic approvals that never park. The host only renders the refusal in its own words (`engine_denial`). SaaS never parks: there is no per-user prompt channel, so the deployment's tool groups are the pre-authorization, a call they allow is recorded as an audited automatic approval under the tenant's own subject and audit stream (`saas_approved`), and anything else is denied.
- Hand the engine the current call posture, including the auto-approve-everything setting and the agent's approvals-off setting. Legacy `autonomy.auto_approve` entries are imported once with `ImportTrustSource2`; they are never replayed during scope registration, so a revoked durable grant cannot be resurrected. Whole-tool grants use `GrantTool2` under the exact user/workspace/agent scope of the parked request. Per-flow grants use `GrantFlowTool2`, `RevokeFlowTool2` and `ListFlowTrust2` and approve only the exact tool in the exact flow. Strict-flow approvals never become global whole-tool grants; forced approvals remain one-time.
- Park the tool-call future when a person must decide, resolve it on a decision (`approve_once` / `approve_always_for_tool` / `approve_always_for_flow` / `deny`), expiry, or channel drop, failing closed in every non-approve path.
- Reserve an approval for execution exactly once, right before the tool runs, and record the call's terminal outcome after it ends (`record_execution`).
- Authenticate the decision: the RPC that carries it is the authenticated human action, so the host issues an opaque ticket for exactly that request and choice, and the module verifies it against the exact pending row (`VerifyHuman`).
- Build safe action summaries (`summarize_action`) before anything leaves the gate; argument redaction is the TinySecurity module's (`security::redact::arguments`, `StructuredArguments` mode).
- Route a thread's yes/no chat reply back to a parked approval (`pending_for_thread` + `parse_approval_reply`).
- On `approve_always_for_tool`, resolve the current call first and then persist its tool grant in TinySecurity; if persistence fails, report a warning and let later calls prompt. On `approve_always_for_flow`, persist the flow trust in TinySecurity.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/security/approval/mod.rs` | Export-focused: module docstring, `pub mod` decls, `pub use` re-exports including the controller-schema pair. |
| `crates/openhuman-core/src/security/approval/engine.rs` | The sealed `ApprovalEngine` port the gate asks (no module types in it), the neutral input/answer types (`CheckInput`, `Posture`, `Variant`, `Checked`), the `PromptSurface` the engine shows a request on, and the `UnavailableEngine` a build without `security-module` gets (it refuses every gated call). |
| `crates/openhuman-core/src/security/approval/native/` | The only real engine, behind `security-module`: `engine.rs` (`NativeEngine`, the module calls and the audit correlation of each phase), `scopes.rs` (subject registration, posture and grants), `audit_stream.rs` (a subject's audit stream, leased by the sandbox planner so its plan events extend the same ledger), `rows.rs` (the document-port store behind `ApprovalStore2` and `AuditSink2`), `callbacks.rs` (what `modules/security_host.rs` serves), `state.rs` (calls, requests and human tickets in flight), `view.rs` (history read from the stored rows, including flow trust and imported legacy requests), `history_import.rs` (one-time import of the legacy `approval.db` into the module: history through `ImportApprovalHistory2`, flow trust through `GrantFlowTool2`, recorded by a marker file). |
| `crates/openhuman-core/src/security/approval/gate.rs` | `ApprovalGate` struct + `DecideMiss`, `DEFAULT_APPROVAL_TTL` (10 minutes) and the shorter `COPILOT_APPROVAL_TTL` and `SUBAGENT_APPROVAL_TTL` (3 minutes, any park inside a sub-agent run), `APPROVAL_UNANSWERED_PHRASE` / `is_unanswered_approval_reason` (TTL expiry vs refusal), `is_detached_request`, the `ApprovalChatContext` / `FlowRunContext` task-locals, `parse_approval_reply`, the `ApprovalGateBootState` record, the `Parked` routing maps and the `CancelGuard`. |
| `crates/openhuman-core/src/security/approval/gate_setup.rs` (`include!`d by [`gate.rs`](./gate.rs), as are the next four) | `ApprovalGate::init_global`/`try_global`, the constructors (`with_engine` is how the gate's own tests run over the scripted engine), the live posture, and the `intercept*` entry points. |
| `crates/openhuman-core/src/security/approval/gate_surface.rs` | `Surface::deliver` (register the waiter and routing, publish the events), the `Parked` map helpers, `Mode` and the call correlation. |
| `crates/openhuman-core/src/security/approval/gate_intercept.rs` | The origin check, engine call, reservation and cancellation-safe bounded park used by the Flow Canvas copilot live-run path. |
| `crates/openhuman-core/src/security/approval/gate_intercept_decision.rs` | `resolve_park`: decision, dropped channel, caller park bound or expiry, including the expire-versus-decide race. |
| `crates/openhuman-core/src/security/approval/gate_state.rs` | `decide` (verifies and applies the decision, resolves the parked future, emits `ApprovalDecided`), `classify_decide_miss`, `record_execution`, `list_pending*` (from the requests parked in this process), `list_recent_decisions`, the flow-trust helpers, and the thread→request routing lookups. The decision side is synchronous by contract (the embedding API, an agent's teardown, the harness's record hook), so it runs the engine's async call on the shared blocking bridge (`run_engine`). |
| `crates/openhuman-core/src/security/approval/store_import.rs` | Reads the legacy `approval.db` (from a scratch copy, never the real file) into typed rows for `native/history_import.rs`. The host keeps no approval documents of its own any more. |
| `crates/openhuman-core/src/security/approval/types.rs` | Serde domain types: `PendingApproval`, `ApprovalAuditEntry`, `ApprovalDecision`, `GateOutcome`, `ExecutionOutcome`. |
| `crates/openhuman-core/src/security/approval/summary.rs` | `summarize_action` (safe-field summary). |
| `crates/openhuman-core/src/security/approval/rpc.rs` | Domain RPC entry points returning `Outcome<T>`: `approval_get_gate_state`, `approval_list_pending`, `approval_list_recent_decisions`, `approval_decide`, `approval_preauthorize_flow`. |
| `crates/openhuman-core/src/security/approval/schemas.rs` | Controller schemas + `handle_*` fns wiring the RPC into the registry. |

## Public surface

Re-exported from [`mod.rs`](./mod.rs):

- Gate: `ApprovalGate`, `ApprovalChatContext`, `FlowRunContext`, `ApprovalTurnContext` (the explicit per-turn chat / copilot / flow-run context every gate check takes), `parse_approval_reply`.
- Summaries: `summarize_action`; redaction: `security::redact::arguments`.
- Types: `PendingApproval`, `ApprovalAuditEntry`, `ApprovalDecision`, `ApprovalSourceContext`, `ExecutionOutcome`, `GateOutcome`.
- Controller registry: `all_approval_controller_schemas`, `all_approval_registered_controllers`.

`ApprovalGate::try_global()` returns `None` when no gate is installed; tools/harness branches treat `None` as "no gating".

## RPC / controllers

Namespace `approval` (registered via `all_approval_registered_controllers`, consumed by `crates/openhuman-core/src/core/all.rs`):

| Method | Inputs | Output |
| --- | --- | --- |
| `approval.list_pending` | none | `pending: PendingApproval[]` |
| `approval.list_recent_decisions` | `limit?: u64` (1-500, default 50) | `decisions: ApprovalAuditEntry[]` |
| `approval.list_tool_trust` | none | `grants: ToolTrustEntry[]` (live grants in the authenticated dispatch tenant's standard scope) |
| `approval.revoke_tool_trust` | `request_id: string` (opaque ID from `list_tool_trust`) | `revoked: bool` |
| `approval.get_gate_state` | none | `state: ApprovalGateBootState` (installed / disabled-by-env / override-ignored / host tag) |
| `approval.decide` | `request_id: string`, `decision: string` (`approve_once` / `approve_always_for_tool` / `approve_always_for_flow` / `deny`) | `decided: PendingApproval` |
| `approval.preauthorize_flow` | `flow_id: string`, `tool_names: string[]` | `result: FlowPreauthorizationResult` (idempotent flow-scoped trust grants; succeeds with `gate_installed=false` when the gate is off) |

`list_pending` / `list_recent_decisions` / `list_tool_trust` return empty (not an error) when the gate is not installed; `decide` and `revoke_tool_trust` error when the gate is absent. `list_pending` lists the requests parked in this process; `list_recent_decisions` is the module's decided rows plus the history older installs wrote. Tool-grant revocation uses TinySecurity's opaque durable row id and is tenant-scoped; the caller cannot name another tenant's scope or tool.

Whole-tool grants can be listed and revoked through the core approval JSON-RPC controllers and the Security settings panel. The approval history panel also displays Jev verdict evidence when present.

## Agent tools

None. This module gates other domains' tools; it owns no tools of its own (no `tools.rs`).

## Events

Published via `crate::core::bus::BUS.publish` with variants from `crate::core::events::DomainEvent` (`crates/openhuman-core/src/core/events.rs`):

- `DomainEvent::ApprovalRequested { request_id, tool_name, action_summary, args_redacted, thread_id, client_id, ... }`: emitted ([`gate_intercept.rs`](./gate_intercept.rs), `Surface::deliver`) when the module asks for the prompt of a request it made durable. Bridged to the `approval_request` web-channel socket event by `ApprovalSurfaceSubscriber` (defined in `crates/openhuman-core/src/web_chat/`).
- `DomainEvent::ApprovalDecided { request_id, tool_name, decision }`: emitted ([`gate_state.rs`](./gate_state.rs)) when a decision is applied.
- `DomainEvent::FlowApprovalRequested { request_id, flow_id, run_id, tool_name, summary }`: emitted (`gate_intercept.rs`) alongside `ApprovalRequested` when the parked call has a `Workflow` origin. It carries no thread/client id, so `ApprovalSurfaceSubscriber` drops it; `openhuman_rpc::server::socketio` broadcasts it as `flow_approval_request` for the Workflows UI.

No `bus.rs` in this module. It only publishes; the subscriber (`ApprovalSurfaceSubscriber`) lives in `crates/openhuman-core/src/web_chat/event_bus.rs`.

## Persistence

The module's rows live in the document store of the current call's storage scope (the acting agent; `local` on a single-user host; refused in SaaS mode with no acting agent), written by `native/rows.rs` when the module calls `ApprovalStore2`:

| Collection | Holds |
| --- | --- |
| `approval_rows` | one document per request id: the module's latest row (JSON) and the fields the host queries (scope, state, expiry, decision time) |
| `approval_mutations` | the content of every accepted write, so an exact retry returns its original receipt |
| `approval_events` / `approval_tails` | the audit events of each subject's stream, contiguous from 1, and the committed tail |
| `approval_streams` | the persisted random audit stream id of each registered subject |
| `approval_meta` | what the user chose in the host's own words (`approve_always_for_tool`), kept for the history view |

`Cas` is a compare-and-swap on the row's version; `Append` accepts a batch that starts at the committed tail or replays one already committed and refuses a gap or a different event at a taken sequence. A restart keeps the history and the audit trail. A request left pending by an earlier launch has no parked future and the module cannot resume another instance's request, so it is never listed or decided again; it expires on its own and is swept (`ExpireApproval2`) the next time its subject is used. Flow trust is one durable `Approved` row per (flow, tool) in `approval_rows`, so it survives a restart with the rest. The legacy `approval.db` is read once, from a scratch copy, when the engine first talks to the loaded module (`native/history_import.rs`): its most recent 500 decided requests become inert history and its flow-trust rows become module grants; a marker file (`approval/module-history-imported`) records that it ran, and the module returns the existing row for anything already imported, so a run cut short is safe to repeat. A request it left pending is not carried over.

## Dependencies

- `crate::core::bus::BUS` + `crate::core::events::DomainEvent` to surface approval prompts/decisions.
- `crate::core::all`: `ControllerFuture` / `RegisteredController` for the controller registry.
- `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`): schema definitions.
- `crate::core::Outcome`: RPC return contract.
- `crate::config::Config`: workspace dir and call posture settings; the boot-time `autonomy.auto_approve` snapshot is only a one-time legacy import source. Durable whole-tool trust is stored by TinySecurity.
- `crate::modules::security` (`approval_proxy`, `approval_init_config`, `audit_init_config`) and `modules/security_host.rs`: the attested module and the callbacks served for it. The host links only `tinysecurity-bus`.
- `crate::security`: `live_policy::current()` for the live "Always allow" list and `POLICY_DENIED_MARKER` for deny reasons.

## Used by

- `crates/openhuman-core/src/core/runtime/bootstrap.rs`: installs the global gate (`ApprovalGate::init_global`) at startup; `core/all.rs` registers the approval RPC controllers.
- `crates/openhuman-core/src/core/all.rs`: registers the controller schemas.
- `crates/openhuman-core/src/agent/tinyagents/middleware.rs` (`ApprovalSecurityMiddleware`, a `wrap_tool` middleware on every turn path): routes external-effect tool calls through the gate before `execute()` and records the terminal outcome.
- `crates/openhuman-core/src/web_chat/`: hands each turn its chat `ApprovalTurnContext`, hosts `ApprovalSurfaceSubscriber`, and routes typed yes/no replies to `approval_decide`.
- `crates/openhuman-core/src/channels/proactive.rs`, `crates/openhuman-core/src/triage/escalation.rs`, `crates/openhuman-core/src/tools/impl/system/install_tool.rs`, `crates/openhuman-core/src/web3/wallet/execution.rs`: interact with the gate / approval types.

## Notes / gotchas

- **Fail-closed 10-minute window is an invariant, not a tunable.** [`gate.rs`](./gate.rs)'s
  `DEFAULT_APPROVAL_TTL` (`Duration::from_secs(60 * 10)`) is what the gate asks for;
  the module only ever shortens it (ten minutes for an operator turn, three for a
  copilot or sub-agent), so `security.approval.ttl_secs` above 600 does not lengthen
  it. A parked call that times out resolves to `Deny`. Do not weaken this default or
  the fail-closed timeout behavior to make a feature work.
- **Interactive only.** An origin that cannot answer never parks: a cron turn's external effect is denied outright, and internal automation runs without asking.
- **Fail-closed everywhere.** An engine that cannot answer (no module, a refused call, a store failure), a reservation that fails, a channel drop, and expiry all return `Deny` (with a `POLICY_DENIED_MARKER`-prefixed reason). The expiry path asks the module to expire the request; when it was decided in the same instant the decision is honored (PR #2367).
- **The waiter is registered before the request can be decided**, inside `Surface::deliver`, which the module calls before it returns `RequireApproval`, so a fast `approval_decide` cannot approve a request while no waiter exists (PR #2149).
- **The host's correlation survives the module's hashing.** The prompt callback finds its call by the call id the host chose, an eight-hex-character value the module's identifier hashing leaves unchanged.
- **`intercept` and `intercept_forced` do not reserve.** Their callers never learn the request id and cannot report how the call ended, so the approval stays as granted instead of leaving a reservation open for ever. The `*_audited*` entry points reserve and the caller records the outcome.
- **Orphan requests are not resumable** (see Persistence). The old gate kept them to show and dismiss; the module's engine refuses to resume another instance's request, so the gate does not list them.
- **`approve_always_for_tool` persistence is the RPC handler's job**, not the gate's: `gate.decide` releases this call first; `rpc::approval_decide` then calls TinySecurity `GrantTool2` for the parked request's exact scope. A failed write warns and later calls prompt again. The module owns the durable grant and applies it on the next check.
- `OPENHUMAN_APPROVAL_GATE=0`/`false` skips installing the gate for CLI, Docker, and library hosts only (`approval_gate_boot_decision` in `crates/openhuman-core/src/core/types.rs`, applied in `core/runtime/bootstrap.rs`); the Tauri shell always installs it and ignores the override. Where honored, `Prompt`-class calls run unprompted.
- **A disabled autonomy policy parks nothing.** With `[security.autonomy] enabled = false` (the shipped default, see `security/README.md`) the module's class gate (`SecurityPolicy::command_class_gate`) answers `Allow` for every class, so the acting tools' `external_effect_with_args` (which asks for `GateDecision::Prompt`) is `false` and the gate is never reached. The gate is still installed and still works the moment the policy is turned on; nothing here is special-cased on the flag.
- A tool name the module reserves (its own effect classes, the human-only tools, `native_*`) cannot be gated as a host tool and is refused rather than approved.

## Tests

- The gate's own behaviour (parking, routing, events, expiry handling) runs over the scripted engine in [`engine_stub.rs`](./engine_stub.rs): [`gate_tests.rs`](./gate_tests.rs) and its siblings, [`engine_tests.rs`](./engine_tests.rs), [`schemas_tests.rs`](./schemas_tests.rs), [`types_tests.rs`](./types_tests.rs), [`store_import_tests.rs`](./store_import_tests.rs).
- What the module itself answers is pinned by the module-backed tests in [`native/engine_tests.rs`](./native/engine_tests.rs), which load the real native library and are `#[ignore]`d unless a released archive (`OPENHUMAN_TEST_SECURITY_RELEASE_HOST`) or a local fixture (`OPENHUMAN_TEST_SECURITY_MODULE`) is selected; the callbacks, store and view have ordinary unit tests beside them.

## Further reading

- [Parent module (`security`)](../README.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
