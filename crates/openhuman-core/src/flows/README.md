# flows

Saved automation workflows: the graphs a user builds on the Workflows canvas,
or that the workflow copilot builds for them in chat. This domain owns the
saved-flow lifecycle (create, edit, enable, run, resume, cancel), the bridge
from trigger events to runs, the agent tools and sub-agents that author and
suggest flows, and the host callbacks that TinyFlows invokes for OpenHuman
services. TinyFlows owns graph validation, compilation, execution, and catalog
behavior; OpenHuman reaches these through [`tinyflows-bus`](../../../../vendor/tinyflows/)
and serves the product-specific callbacks.

Callers are the JSON-RPC/CLI surface (`flows.*`, used by the Workflows UI), the
event bus (cron ticks and Composio triggers), and agents that hold the flow
tools.

## How it works

### The pieces

```text
   Workflows UI        chat agent / workflow_builder     event bus
        |                      |                            |
        | flows.* RPC          | flow tools                 | schedule tick,
        v                      v                            | Composio trigger
   schemas.rs --------->  ops/*  <-----------------  bus/trigger.rs
                               |  |
                 policy,      |  |  trigger dispatch
                 RPC mapping  |  |
                               v  v
                       TinyFlows TinyBus contracts
                  Authoring / Catalog / Runs / Portability
                         |                         ^
              CatalogStorage callback             | CapsHost callbacks
                         |                         |
                  host storage             OpenHuman services: harness,
                                          Composio, native tools, memory,
                                          HTTP and approval policy
```

`ops/` is the host business layer. RPC handlers, agent tools, and the trigger
subscriber call into it. It calls TinyFlows through bus contracts and applies
OpenHuman policy; TinyFlows calls back through the host's scoped storage and
capability objects.

### Saving a flow

A graph arrives as raw JSON, from the canvas, an import, a draft promotion or
an agent tool. OpenHuman calls TinyFlows' `Authoring.ValidateGraph` and
`Authoring.ImportGraph` bus members for migration, validation and import
normalization. Validation passes a per-call `ResolveWorkflow` callback so
compatibility checks can inspect saved children in the caller's profile.

Agent-authored graphs pass one more stack, `run_builder_gates` in
[`ops/builder_gates.rs`](./ops/builder_gates.rs), which calls TinyFlows'
`Authoring.BuilderFailures` with a scoped `AuthoringHost`. It is the single definition of the author-time hard
gates, run in increasing cost order and short-circuiting on the first failure:
engine compatibility, binding resolvability, agent-ref resolvability,
connection refs ([`host_gates/connections.rs`](./host_gates/connections.rs): a `composio:<toolkit>:<id>` ref
must name a connected account of the same toolkit as the slug), tool contracts
([`host_gates/tool_contracts.rs`](./host_gates/tool_contracts.rs): the slug must exist in the live Composio catalog
and every required arg must be present), and required-arg resolvability.
`propose_workflow`, `revise_workflow`, `edit_workflow`, `save_workflow`, and the
`strict` create/update RPC path all route through it, so agent saves and UI
saves cannot validate differently. Best-effort output-field, payload-envelope,
and split-path findings come from TinyFlows' `Authoring.WiringWarnings` through
[`ops/wiring_warnings.rs`](./ops/wiring_warnings.rs), with a scoped host
`LookupToolContract` callback backed by the Composio catalog. Missing required
args remain the responsibility of the hard `validate_tool_contracts` gate;
they are not repeated as advisory warnings.

`flows_create` (in [`ops/definitions.rs`](./ops/definitions.rs)) then applies two server-side rules
regardless of what the caller asked for:

1. A graph whose trigger fires without a human (`schedule`, `app_event`,
   `webhook`) is created disabled. The user has to enable it explicitly.
2. A graph with outbound side-effect nodes (`tool_call`, `http_request`,
   `code`) is forced to `require_approval = true`.

`flows_update` applies the same rules to later saves: it forces approval on
side-effect graphs, and when a graph's trigger changes from manual (or none)
to automatic it turns the flow off. `flows_duplicate` always produces a
disabled, unbound copy, and `flows_draft_promote` runs the create/update path.
No save path hands the user an armed, unattended automation. Each successful write publishes `DomainEvent::FlowChanged` so an
open canvas refetches. Updates use optimistic concurrency: a stale write comes
back as a `{ code: "version_conflict", ... }` error the UI can offer to reload.
Every graph change is kept as a revision (`flows_get_history`,
`flows_rollback`).

Drafts are separate, non-live working copies owned by the TinyFlows Catalog
service ([`draft_store.rs`](./draft_store.rs), [`ops/drafts.rs`](./ops/drafts.rs)). Promoting a
draft runs the same create/update gates as a normal save.

### Enabling and triggers

`flows_set_enabled` (in [`ops/triggers.rs`](./ops/triggers.rs)) binds or unbinds the flow's
automatic dispatch based on its trigger node:

| Trigger kind | What enabling does |
| --- | --- |
| `schedule` | Arms a `JobType::Flow` cron job with `cron::add_flow_schedule_job`; disabling calls `cron::remove_job`. |
| `app_event` | Nothing to bind. The trigger subscriber matches Composio trigger events against enabled flows at dispatch time. |
| `webhook` | Not implemented. Enabling logs an actionable warning; incoming webhook requests are observed but never start a run. |
| `manual` and others | No automatic dispatch. |

`flows_update` rebinds a schedule when its normalized trigger kind or typed
schedule changes, so a new cron expression takes effect without rebinding for
pinned-input or unrelated graph edits. At boot,
`reconcile_schedule_triggers_on_boot` (called from [`core/runtime/services.rs`](../core/runtime/services.rs))
re-arms schedules for every enabled flow.

```text
 cron scheduler                 Composio webhook relay
      | TriggerSink (flow_id)          | ComposioTriggerReceived{toolkit, slug}
      v                                v
 +--------------------------------------------------+
 | bus::FlowTriggerSubscriber                       |
 |  - load flow, check enabled + trigger kind       |
 |  - app_event: match toolkit/slug on every flow   |
 |  - try_acquire_dispatch(flow_id): skip if a      |
 |    trigger-driven run of this flow is in flight  |
 |  - spawn_run -> tokio task -> ops::flows_run     |
 +--------------------------------------------------+
```

### Running a flow

There are two entry points in [`ops/run.rs`](./ops/run.rs). `flows_run` blocks
until the TinyFlows run parks for approval or reaches a terminal result.
`flows_run_detached` performs the same host preflight, starts the module run,
and returns `{ run_id, status: "running", detached: true }` immediately. The
existing RPC result for a blocking run remains
`{ output, pending_approvals, thread_id }`.

```text
 OpenHuman preflight: owner-scoped workflow lookup, host compatibility,
 inference readiness, trusted input envelope
        |
 runs_adapter::start_run
   retain stable CatalogStorage + per-run CapsHost registrations
   Runs.StartRun(graph, input, inputs, timeout, workflow origin)
        |
 one owner-scoped PollRun cursor
   started / step_started / step / graph observations / agent progress
   -> OpenHuman start/progress events and host Langfuse export
        |
   parked -> keep bindings alive, notify pending approval, no FlowRunFinished
   finished -> publish FlowRunFinished once, map the legacy RPC result
```

The module owns graph compilation, input validation, execution, run rows,
checkpoints, cancellation, and dedup settlement. OpenHuman keeps inference
readiness and compatibility preflight, policy and approval callbacks, the
trusted workflow origin, event projection, and Langfuse export. `input` is the
free-form trigger payload (reachable in expressions as `=run.trigger.…`);
`inputs` fills the flow's declared inputs (`=inputs.<name>`).

A run that reaches a gated node parks as `pending_approval`; the module keeps
its durable snapshot and checkpoint for `flows_resume` to continue. OpenHuman
checks host compatibility and binds a fresh CapsHost to the same owner-scoped
CatalogStorage address. `flows_approval_manifest` computes, ahead of time,
every trust key a fully pre-authorized run would need, so the save-and-enable
card can ask once instead of parking node by node.

`flows_cancel_run` (in [`ops/run_management.rs`](./ops/run_management.rs)) calls
Runs.CancelRun through the run's exact CatalogStorage owner address. The
module settles active, parked, and orphaned runs. Boot recovery and parked-run
expiry likewise call typed Runs operations and project only rows the module
actually changed.

### After a run

Two host bus subscribers are constructed in
[`core/runtime/subscribers.rs`](../core/runtime/subscribers.rs).
`FlowRunDigestSubscriber` ([`bus/run_digest.rs`](./bus/run_digest.rs)) writes a
short digest of each successful run into memory, tagged `flow:<id>` and
`flow_run_digest`, keeping the newest 50 per flow. `FlowTriggerSubscriber` is
the trigger bridge described above. The TinyFlows module settles `dedup` node
state as part of its winning run lifecycle operation.

### Authoring and discovery agents

Two built-in sub-agents live under `../agent_registry/agents/`, each with an
`agent.toml` and a synchronous `prompt::build`. Their archetype text is fetched
from the TinyFlows Copilot bus by async host entrypoints, then supplied to the
renderer. Both are registered by path from the `BUILTINS` slice in
[`agent_registry/agents/loader.rs`](../agent_registry/agents/loader.rs):

- `workflow_builder` is the authoring copilot. `flows_build` (in
  [`ops/builder.rs`](./ops/builder.rs)) runs one turn of it from a structured request (create,
  revise, repair, build), streams progress onto a chat thread when asked
  ([`ops/streaming.rs`](./ops/streaming.rs)), and returns the `workflow_proposal` it produced plus its
  final text. [`ops/trail_off.rs`](./ops/trail_off.rs) detects a turn that ended in a question or a
  gate failure instead of a proposal. The overall bound is 600 seconds.
- `flow_discovery` is a read-only scout. `flows_discover` (in [`ops/discovery.rs`](./ops/discovery.rs))
  runs it over the user's data; it ends by calling `suggest_workflows`, and the
  suggestions are stored for the UI (`flows_list_suggestions`,
  `flows_dismiss_suggestion`, `flows_mark_suggestion_built`).

The human-in-the-loop rule is enforced by the tools, not the prompt. No agent
tool enables a flow. `propose_workflow`, `revise_workflow`, `edit_workflow` and
`validate_workflow` never persist. `create_workflow` and `duplicate_flow`
always produce a disabled flow, and `save_workflow` never sets `enabled` or
`require_approval` (it can auto-disable a flow whose trigger turns automatic).
`dry_run_workflow` runs a draft against `tinyflows`' mock capabilities, so no
real effect can fire. `get_tool_output_sample` is the one builder tool that
makes a real Composio call, limited to Read-scope actions on a connected
toolkit. `resume_flow_run` is `Execute`-gated because resuming lets approved
nodes fire.

The long authoring manual is owned by TinyFlows Copilot. With the `flows` and
`skills` features and runtime components enabled, OpenHuman requests its
`flow-authoring` files through `tinyflows_bus::copilot::ListResources` and
materializes them through TinySkills under a separate scoped builtin root.
Binding rules stay in the agent prompt; reference material the model looks up
goes in the skill.

### Flow memory

Memory has no namespaces, so a flow's memory is the set of items tagged
`flow:<id>` (plus `flows`). The tag helpers in [`memory_tools.rs`](./memory_tools.rs) (`flow_tag`,
`flow_key_tag`, `flow_meta`, `flow_filter`, `cross_flow_filter`,
`remember_keyed`, `forget_matching`, `FLOWS_TAG`) are re-exported from [`mod.rs`](./mod.rs)
so the digest subscriber, `flows_delete`, the `memory` node adapter
served through CapsHost and the agent tools all tag the same way.

### Storage

The catalog (`store.rs`), drafts (`draft_store.rs`), run rows, per-flow state,
and checkpoints are owned by TinyFlows' module services and the document port.
OpenHuman registers a scope-bound CatalogStorage callback per acting storage
scope and retains its stable address with each live or approval-parked run.
There is one storage path: the installed host storage backend, which defaults
to local SQLite for desktop use. Startup migrates the module's catalog and
schedule stores for every discovered storage scope before services start. The
legacy files TinyFlows used to write (`flows/flows.db`, `flows/drafts/`,
`flows/checkpoints.db`) are read through bounded readers and replayed through
Catalog bus requests; they are never modified. A clean import marker is written
only after all batches succeed. If the import is incomplete, the local flow
scope stays paused and retries on the next boot; chat, login, cron and other
storage scopes continue. The unbooted local-storage fallback migrates the
Catalog schema on first use but deliberately does not import legacy files.
When the acting scope cannot be resolved (SaaS mode with no acting agent), flow
state fails every call rather than falling back to a shared bucket.

## Layout

Top level:

| Path | What it does |
| --- | --- |
| `mod.rs` | Module declarations and re-exports: node contracts and flow-memory helpers. |
| [`ops.rs`](./ops.rs), `ops/` | Business operations returning `Outcome<T>`. See the table below. |
| [`schemas.rs`](./schemas.rs), `schemas/`, [`schemas_handlers.rs`](./schemas_handlers.rs) | Controller schemas (split into `builder_schemas.rs`, `definition_schemas.rs`, `draft_schemas.rs`, `run_schemas.rs`) and thin handlers that load config, read params and call `ops::flows_*`. |
| [`store.rs`](./store.rs) | Translates host flow operations to TinyFlows Catalog bus requests; the module owns catalog persistence through the scoped `CatalogStorage` callback. |
| `draft_store.rs` | Compatibility adapter for draft requests over TinyFlows' Catalog contract. |
| [`bus.rs`](./bus.rs), `bus/` | The trigger and run-digest subscribers. |
| [`runs_adapter.rs`](./runs_adapter.rs), `runs_adapter/` | Scope-bound CatalogStorage and CapsHost ownership, Runs Start/Poll/Cancel calls, cursor projection, and legacy RPC result mapping. |
| [`tools.rs`](./tools.rs) | `ProposeWorkflowTool` (`propose_workflow`) and `RunFlowTool` (`run_flow`). |
| [`builder_host.rs`](./builder_host.rs) | Scope-bound `CopilotHost` callback over TinyBus. The compiled module owns the authoring tools; the callback binds their draft, flow, gate, agent, and suggestion ports to OpenHuman operations for one invocation. |
| [`crate::integrations::composio::builder_tools`](../integrations/composio/builder_tools/mod.rs) | Composio-backed workflow-builder reads and catalog ranking. The mixed connection picker is composed by the flow host through a provider. |
| `memory_tools.rs` | `FlowMemoryRecallTool`, `FlowMemoryRememberTool`, and the flow-memory tag helpers. |
| `../agent/registry/agents/` | `workflow_builder/` and `flow_discovery/` built-in definitions and prompt builders. |
| [`catalogue.rs`](./catalogue.rs) | `flow_entries` lists saved flows as `Workflow` entries with `WorkflowScope::Flow`, so the skill catalogue and `skill_search` show one list. |
| [`node_contracts.rs`](./node_contracts.rs) | Host overlay on `Catalog.NodeKinds` returned over TinyBus (which `tool_call` slugs resolve to Composio or native `oh:` tools, which trigger kinds dispatch here). |
| [`host_gates/`](./host_gates/) | OpenHuman stages served through the scoped TinyFlows `AuthoringHost` callback. |
| [`host_gates/graph_json.rs`](./host_gates/graph_json.rs) | Read-only traversal of TinyFlows' normalized graph JSON for host-owned policy checks. |
| `../skills/bundled/flow_authoring.rs` | Adapts Copilot `ListResources` to TinySkills materialization and scoped skill discovery. |

`ops/`, grouped by role:

| Group | Files |
| --- | --- |
| Definitions | `definitions.rs` (create, get, list, delete, duplicate), `updates.rs` (update, history, rollback), `validation.rs` (validate, import, migrate), `drafts.rs` |
| Triggers | `triggers.rs` (set_enabled, bind/unbind, boot reconcile) |
| Runs | `run.rs` (preflight and entry points), `execution.rs` (approval notification and Langfuse export), `resume.rs`, `run_management.rs` (cancel, list, prune), `inference_readiness.rs`; module lifecycle adapter in `runs_adapter.rs` |
| Gates and manifests | `builder_gates.rs`, `approval_manifest.rs` |
| Copilot and scout | `builder.rs`, `builder_toolset.rs`, `streaming.rs`, `trail_off.rs`, `discovery.rs` |
| Catalog reads | `catalog.rs` (tool catalog search, tool contract), `connections.rs` (connection picker, required connections) |

## Key types and entry points

- `ops::flows_run` / `ops::flows_run_detached` (`ops/run.rs`): start a run,
  blocking or fire-and-forget.
- `ops::flows_create` / `ops::flows_update` (`ops/definitions.rs`,
  [`ops/updates.rs`](./ops/updates.rs)): the save paths that enforce born-disabled and forced
  approval.
- `Authoring.ValidateGraph` / `Authoring.ImportGraph`: module-owned graph
  migration, validation and import over TinyBus.
- `run_builder_gates` (`ops/builder_gates.rs`): the agent-authored hard-gate
  stack.
- `runs_adapter::RunBindings`: retains the same owner-scoped CatalogStorage and
  per-run CapsHost registration from admission through terminal settlement or
  approval parking. Resume uses a fresh CapsHost registration and the exact
  same stable storage address.
- `bus::FlowTriggerSubscriber` ([`bus/trigger.rs`](./bus/trigger.rs)): the trigger-to-run bridge.
- Workflow, run, draft, and validation records cross the host boundary through
  `tinyflows_bus` Catalog, Runs, and Authoring contracts. `FlowConnection` is
  OpenHuman's secret-free picker projection in `ops/connections.rs`; graph
  payloads stay normalized JSON at the host boundary.

## RPC / CLI surface

All methods live in the `flows` namespace (`openhuman.flows_<name>` over
JSON-RPC), registered through `all_flows_registered_controllers()` in
[`core/all.rs`](../core/all.rs).

| Group | Methods |
| --- | --- |
| Definitions | `create`, `get`, `list`, `update`, `delete`, `duplicate`, `import`, `validate`, `get_history`, `rollback`, `set_enabled` |
| Runs | `run`, `run_detached`, `resume`, `cancel_run`, `get_run`, `list_runs`, `list_all_runs`, `prune_runs`, `approval_manifest` |
| Builder | `build`, `build_cancel`, `search_tool_catalog`, `get_tool_contract`, `list_connections`, `required_connections` |
| Discovery | `discover`, `list_suggestions`, `dismiss_suggestion`, `mark_suggestion_built` |
| Drafts | `draft_create`, `draft_get`, `draft_update`, `draft_list`, `draft_delete`, `draft_promote` |

Agent tools: `core`'s [`tools/ops.rs`](../tools/ops.rs) adds 27 flows tools to the tool list
(`propose_workflow`, `run_flow`, the 22 builder tools, `suggest_workflows`,
`flow_memory_recall`, `flow_memory_remember`), each behind the `flows` feature.

## Boundaries

- The TinyFlows module owns graph migration and validation, Catalog and Runs
  behavior, persistence schemas, execution, and Copilot prompts. OpenHuman calls
  the `tinyflows-bus` contracts and supplies scoped CatalogStorage and CapsHost
  callbacks. Keep engine, catalog and module behavior in the TinyFlows owner
  repository; keep OpenHuman policy and host adapters here.
- `cron/` owns scheduling. This domain only arms and removes `JobType::Flow`
  jobs; the scheduler fires them through the schedule runtime's `TriggerSink`.
- [`integrations/composio/`](../integrations/composio/) owns the Composio catalog, connections and trigger
  ingestion. Flows reads them for gates, pickers and `tool_call` execution.
- `memory/` owns storage. Flows only tags items.
- The approval gate (`approval::gate`) decides when a node parks. The approval
  manifest mirrors that logic and never re-implements it.
- The skill catalogue types (`Workflow`, `WorkflowScope`) and `BundledSkill`
  belong to `skills/`.
- Webhook-triggered dispatch is not implemented in this domain.

## Gotchas

- The family is gated as a leaf, not a facade. `pub mod flows;` in `lib.rs` sits
  behind `#[cfg(feature = "flows")]` and there is no `stub.rs`. Every outside
  reference is a registration site (`core::all`, the subscribers in
  `core::runtime::subscribers`, the boot sweeps in `core::runtime::services`,
  the tool list in `tools::ops`, the `BUILTINS` entries, the catalogue extension
  in [`agent/session_host/builder/factory.rs`](../agent/session_host/builder/factory.rs)), and a registration site wants
  absence when the feature is off. A stub would make `flows.*` a known method
  that fails at runtime. If an always-compiled domain ever gains a real `use
  crate::flows::...`, convert to the facade-plus-stub shape used by `voice/`.
- StartRun and ResumeRun must receive the trusted context and the typed
  owner-scoped CapsHost and CatalogStorage addresses. Keep their registrations
  alive until PollRun reports terminal settlement; dropping them after an
  admission acknowledgement breaks active or parked callbacks.
- `flows_resume` must keep its pending-node check. The engine does not enforce
  the `approvals` argument.
- Run rows, checkpoints, event history, and dedup settlement are module-owned;
  OpenHuman projects PollRun events and keeps the established RPC response.

## Tests

Tests sit beside their modules as `*_tests.rs` files ([`ops_tests.rs`](./ops_tests.rs),
[`bus_tests.rs`](./bus_tests.rs), [`schemas_tests.rs`](./schemas_tests.rs), and the builder tool and agent prompt suites).
`storage::import::tinyflows_tests` covers owner migrations and read-only legacy
file replay; Catalog behavior belongs to the TinyFlows module's own suite.

```bash
cargo test -p openhuman flows::
pnpm debug rust flows
```

Related docs: [docs/gitbooks/en/features/workflows.md](../../../../docs/gitbooks/en/features/workflows.md)
(user-facing feature), [docs/gitbooks/en/developing/architecture/flows-on-tinyflows.md](../../../../docs/gitbooks/en/developing/architecture/flows-on-tinyflows.md)
(module boundary, run lifecycle, storage migration, and host policy), and
[../cron/README.md](../cron/README.md) (`JobType::Flow`).

## Further reading

- [Parent module README](../../README.md)
- [tinyflows](../../../../vendor/tinyflows/README.md)
