# flows

Saved automation workflows: the graphs a user builds on the Workflows canvas,
or that the workflow copilot builds for them in chat. This domain owns the
saved-flow lifecycle (create, edit, enable, run, resume, cancel), the bridge
from trigger events to runs, the agent tools and sub-agents that author and
suggest flows, and the adapters that let the vendored `tinyflows` engine call
real OpenHuman services. The engine itself (graph model, validation,
compilation, execution) lives in [`vendor/tinyflows/`](../../../../vendor/tinyflows/).

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
          validate, gate,      |  |  compile + run
          persist              |  |
                               v  v
   store.rs / draft_store.rs       tinyflows engine (vendor/tinyflows)
   (FlowCatalogDocuments on the           |
    storage ports)                       | capability traits
                                         v
                              tinyflows/caps  (agent, llm, tools,
                              http, code, memory, state store)
                                         |
                                         v
                       OpenHuman services: harness, Composio,
                       native tools, memory, approval gate
```

`ops/` is the business layer. Everything else either calls into it (RPC
handlers, agent tools, the trigger subscriber) or is called by it (the store,
the engine, the capability seam).

### Saving a flow

A graph arrives as raw JSON, from the canvas, an import, a draft promotion or
an agent tool. `tinyflows::authoring::validate_and_migrate_graph` runs
it through `tinyflows::migrate::migrate` to upgrade older schemas, deserializes
it into a `WorkflowGraph`, and rejects anything `tinyflows::validate::validate`
refuses. Engine-compatibility checks map topology problems onto
`FlowValidationError`.

Agent-authored graphs pass one more stack, `run_builder_gates` (a `tinyflows::gates::BuilderGateHost` over `builder_failures`) in
[`ops/builder_gates.rs`](./ops/builder_gates.rs). It is the single definition of the author-time hard
gates, run in increasing cost order and short-circuiting on the first failure:
engine compatibility, binding resolvability, agent-ref resolvability,
connection refs ([`ops/connection_ref_gate.rs`](./ops/connection_ref_gate.rs): a `composio:<toolkit>:<id>` ref
must name a connected account of the same toolkit as the slug), tool contracts
([`ops/tool_contract_gate.rs`](./ops/tool_contract_gate.rs): the slug must exist in the live Composio catalog
and every required arg must be present), and required-arg resolvability.
`propose_workflow`, `revise_workflow`, `edit_workflow`, `save_workflow`, and the
`strict` create/update RPC path all route through it, so agent saves and UI
saves cannot validate differently. Softer findings come back as warnings
instead: [`ops/wiring_warnings.rs`](./ops/wiring_warnings.rs) flags unwired required args and bindings to
output fields the action does not produce.

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

Drafts are separate, non-live working copies stored as files under
`<workspace>/flows/drafts/` ([`draft_store.rs`](./draft_store.rs), [`ops/drafts.rs`](./ops/drafts.rs)). Promoting a
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

`flows_update` rebinds a schedule when an enabled flow's trigger kind or config
changes, so a new cron expression takes effect. At boot,
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

There are two entry points in [`ops/run.rs`](./ops/run.rs). `flows_run` blocks until the run
settles; `flows_run_detached` validates synchronously, then spawns the run and
returns `{ run_id, status: "running", detached: true }` at once. The detached
form exists because the harness caps one tool call at 120 seconds and a flow
with a research agent node easily runs longer. The UI "Run" button and the
`run_flow` agent tool use the detached path; the copilot polls `get_flow_run`.

Both share one body, `run_flow_body` in [`ops/execution.rs`](./ops/execution.rs):

```text
 prepare_flow_run      validate declared inputs, compile-check, mint thread_id
        |
 run_registry::register(thread_id)   cancel token + guard (before the id
        |                            is visible, so a cancel never races it)
 start_flow_run_row    insert flow_runs row, status "running"
 FlowRunStarted        published on the bus
        |
 run_flow_body
   RunRowFinalizer     drop guard: writes "interrupted" if the future dies
   validate_inference_readiness   fail fast when no AI provider works
   tinyflows::compiler::compile
   build_capabilities + open_flow_checkpointer
   run_with_checkpointer_journaled_observed   (raced against cancel token)
        |  FlowRunObserver persists each finished step to the run row
        v
 finalize: completed | completed_with_warnings | pending_approval |
           failed | cancelled
   record summary on the flow, Langfuse export (if usage sharing is on),
   CoreNotification with an "approve" action when approvals are pending
        |
 FlowRunFinished  ->  FlowRunDigestSubscriber, DedupCommitSubscriber
```

The whole run is scoped under
`AgentTurnOrigin::TrustedAutomation { source: Workflow { require_approval } }`
(`workflow_origin`), whatever started it. The trust argument is about the
saved, validated graph, not the caller. `input` is the free-form trigger
payload (reachable in expressions as `=run.trigger.…`); `inputs` fills the
flow's declared inputs (`=inputs.<name>`), and a bad declared input is rejected
before any run row exists.

A run that reaches a gated node without trust parks as `pending_approval`. Its
state lives in a durable SQLite checkpoint (`checkpoints.db`), so it survives a
restart. `flows_resume` (in [`ops/resume.rs`](./ops/resume.rs)) continues it, but only after
checking that the supplied `approvals` name at least one of the row's actually
pending node ids. The engine treats any resume call as approval, so this guard
is the host's enforcement. `flows_approval_manifest` computes, ahead of time,
every trust key a fully pre-authorized run would need, so the save-and-enable
card can ask once instead of parking node by node.

`flows_cancel_run` (in [`ops/run_management.rs`](./ops/run_management.rs)) signals an in-flight run
through `run_registry::cancel` and lets that run's own cancel arm write the
terminal row. For a parked or orphaned run it writes `cancelled` itself and
drops the checkpoint. Two sweeps clean up after crashes:
`sweep_orphaned_running_runs_on_boot` marks rows left `running` by a dead
process, and `sweep_expired_parked_runs` expires old parked runs.

### After a run

Three bus subscribers, all constructed in [`core/runtime/subscribers.rs`](../core/runtime/subscribers.rs), react
to run events. `FlowRunDigestSubscriber` ([`bus/run_digest.rs`](./bus/run_digest.rs)) writes a short
digest of each successful run into memory, tagged `flow:<id>` and
`flow_run_digest`, keeping the newest 50 per flow. `DedupCommitSubscriber`
([`bus/dedup_commit.rs`](./bus/dedup_commit.rs)) settles the `dedup` node's exactly-once contract: on
success it folds each node's tentative key set into its committed set; on
failure it drops the tentative keys so the items are retried. Commits are
serialized per flow. `FlowTriggerSubscriber` is the trigger bridge described
above.

### Authoring and discovery agents

Two built-in sub-agents live under `agents/`, each with an `agent.toml` and a
`prompt::build`, registered by path from the `BUILTINS` slice in
[`agent/registry/agents/loader.rs`](../agent/registry/agents/loader.rs):

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

The long authoring manual is a bundled skill ([`skills/mod.rs`](./skills/mod.rs), needs both the
`flows` and `skills` features): the portable `flow-authoring` pages from
`tinyflows-copilot`, registered with OpenHuman's native skill runtime. Binding
rules stay in the agent prompt; reference material the model looks up goes in
the skill.

### Flow memory

Memory has no namespaces, so a flow's memory is the set of items tagged
`flow:<id>` (plus `flows`). The tag helpers in [`memory_tools.rs`](./memory_tools.rs) (`flow_tag`,
`flow_key_tag`, `flow_meta`, `flow_filter`, `cross_flow_filter`,
`remember_keyed`, `forget_matching`, `FLOWS_TAG`) are re-exported from [`mod.rs`](./mod.rs)
so the digest subscriber, `flows_delete`, the `memory` node adapter
(`tinyflows::memory_adapter::OpenHumanMemory`) and the agent tools all tag the
same way.

### Storage

The catalog (`store.rs`), drafts (`draft_store.rs`), per-flow state and the run
checkpointer are on the document port, in the acting agent's scope:
`tinyflows_drivers::catalog::FlowCatalogDocuments`, `FlowStateDocuments`
(through `tinyflows/state.rs`'s `FlowState`, which the engine's `StateStore`
and the dedup settlement share) and `DriverCheckpointer`. tinyflows has no
SQLite store of its own any more, so there is one path whatever the storage URL
says: the installed backend, else a SQLite document database of the catalog's own
(`<workspace>/flows/catalog.db`). The files tinyflows used to write
(`flows/flows.db`, `flows/drafts/`, `flows/checkpoints.db`) are imported
read-only once, at boot for an installed backend (`storage::import::tinyflows`)
and on first use otherwise, and are never written again. When the scope cannot
be resolved (SaaS mode with no acting agent) flow state fails every call rather
than falling back to a shared bucket.

## Layout

Top level:

| Path | What it does |
| --- | --- |
| `mod.rs` | Module declarations and re-exports: model types from `tinyflows_catalog`, node contracts, store helpers for the seam, flow-memory helpers. |
| [`ops.rs`](./ops.rs), `ops/` | Business operations returning `Outcome<T>`. See the table below. |
| [`schemas.rs`](./schemas.rs), `schemas/`, [`schemas_handlers.rs`](./schemas_handlers.rs) | Controller schemas (split into `builder_schemas.rs`, `definition_schemas.rs`, `draft_schemas.rs`, `run_schemas.rs`) and thin handlers that load config, read params and call `ops::flows_*`. |
| [`store.rs`](./store.rs) | Binds `FlowCatalogDocuments` to this host's storage (`scoped`, `documents`). Every function only substitutes the catalog; anything else in a body is host policy leaking into persistence. |
| `draft_store.rs` | Same binding for the catalog's drafts. |
| [`bus.rs`](./bus.rs), `bus/` | The three subscribers: `trigger.rs`, `run_digest.rs`, `dedup_commit.rs`. |
| [`tools.rs`](./tools.rs) | `ProposeWorkflowTool` (`propose_workflow`) and `RunFlowTool` (`run_flow`). |
| [`builder_host.rs`](./builder_host.rs) | `FlowsHost` and `OverlayNodeKinds`: the host implementation of the `tinyflows_copilot::tools` ports (drafts, flows, gates, agents, suggestions, node kinds). The authoring tools themselves, including `suggest_workflows`, live in `tinyflows-copilot`. |
| [`builder_tools.rs`](./builder_tools.rs), `builder_tools/` | The Composio-bound `workflow_builder` reads: `connection_reads.rs`, `catalog_search.rs`, `tool_contract.rs`. |
| `memory_tools.rs` | `FlowMemoryRecallTool`, `FlowMemoryRememberTool`, and the flow-memory tag helpers. |
| `agents/` | `workflow_builder/` and `flow_discovery/` built-in agent definitions and prompt builders. |
| [`catalogue.rs`](./catalogue.rs) | `flow_entries` lists saved flows as `Workflow` entries with `WorkflowScope::Flow`, so the skill catalogue and `skill_search` show one list. |
| [`node_contracts.rs`](./node_contracts.rs) | Host overlay on `tinyflows::catalog`'s node-kind contracts (which `tool_call` slugs resolve to Composio or native `oh:` tools, which trigger kinds dispatch here). |
| `skills/` | Registers the bundled `flow-authoring` skill. |
| `tinyflows/` | The capability seam. See [tinyflows/README.md](tinyflows/README.md). |

`ops/`, grouped by role:

| Group | Files |
| --- | --- |
| Definitions | `definitions.rs` (create, get, list, delete, duplicate), `updates.rs` (update, history, rollback), `validation.rs` (validate, import, migrate), `drafts.rs` |
| Triggers | `triggers.rs` (set_enabled, bind/unbind, boot reconcile) |
| Runs | `run.rs` (entry points), `execution.rs` (shared body, origin, notifications, Langfuse export), `run_rows.rs` (row writes, `RunRowFinalizer`), `resume.rs`, `run_management.rs` (cancel, list, prune, sweeps), `inference_readiness.rs` |
| Gates and warnings | `builder_gates.rs`, `connection_ref_gate.rs`, `tool_contract_gate.rs`, `wiring_warnings.rs`, `approval_manifest.rs` |
| Copilot and scout | `builder.rs`, `builder_toolset.rs`, `streaming.rs`, `trail_off.rs`, `discovery.rs` |
| Catalog reads | `catalog.rs` (tool catalog search, tool contract), `connections.rs` (connection picker, required connections) |

## Key types and entry points

- `ops::flows_run` / `ops::flows_run_detached` (`ops/run.rs`): start a run,
  blocking or fire-and-forget.
- `ops::flows_create` / `ops::flows_update` (`ops/definitions.rs`,
  [`ops/updates.rs`](./ops/updates.rs)): the save paths that enforce born-disabled and forced
  approval.
- `tinyflows::authoring::validate_and_migrate_graph`: the one
  structural-validation door.
- `run_builder_gates` (`ops/builder_gates.rs`): the agent-authored hard-gate
  stack.
- `RunRowFinalizer` ([`ops/run_rows.rs`](./ops/run_rows.rs)): drop guard that keeps a run row from
  being wedged at `running`.
- `tinyflows::build_capabilities` / `tinyflows::open_flow_checkpointer`
  ([`tinyflows/caps/`](./tinyflows/caps/)): wire the engine to OpenHuman for one run.
- `bus::FlowTriggerSubscriber` ([`bus/trigger.rs`](./bus/trigger.rs)): the trigger-to-run bridge.
- Model types, re-exported from `tinyflows_catalog` and not owned here: `Flow`,
  `FlowConnection`, `FlowDraft`, `FlowImport`, `FlowRevision`, `FlowRun`,
  `FlowRunStep`, `FlowRunTrigger`, `FlowSuggestion`, `FlowValidation`,
  `FlowValidationError`, `SuggestionStatus`, `DraftOrigin`, plus the `types`,
  `run_registry`, `build_registry` and `n8n_import` modules.

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

- `vendor/tinyflows` (tinyflows repo) owns the workflow model, migration,
  validation, compilation, and the run engine with its in-crate state-graph
  runtime. `tinyflows-catalog` owns the saved-flow types and registries,
  `tinyflows-drivers` owns the document layout for flows, drafts, runs and
  checkpoints, and `tinyflows-copilot` owns the authoring manual and pure text heuristics.
  Fix engine and storage behavior there, not here.
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
- Register the cancel token before the run id becomes observable. Both run
  entry points do this; moving registration into the spawned task reopens a race
  where a cancel writes `cancelled` while the run still fires its side effects.
- `flows_resume` must keep its pending-node check. The engine does not enforce
  the `approvals` argument.
- `kv_get`, `kv_set` and `upsert_flow_run_step` are public only so the seam
  (`SqliteStateStore`, `FlowRunObserver`) can reach the store from the sibling
  `tinyflows` module.

## Tests

Tests sit beside their modules as `*_tests.rs` files ([`ops_tests.rs`](./ops_tests.rs),
[`bus_tests.rs`](./bus_tests.rs), [`schemas_tests.rs`](./schemas_tests.rs), the builder tool and agent prompt tests,
and the seam's own suite under `tinyflows/`). `store.rs`'s tests cover the
storage resolution and the import of the legacy files; the catalog's behavior is
covered in `tinyflows-drivers`.

```bash
cargo test -p openhuman flows::
pnpm debug rust flows
```

Related docs: [docs/gitbooks/en/features/workflows.md](../../../../docs/gitbooks/en/features/workflows.md)
(user-facing feature), [docs/gitbooks/en/developing/architecture/flows-on-tinyagents.md](../../../../docs/gitbooks/en/developing/architecture/flows-on-tinyagents.md)
(run pipeline and security model; it still describes lowering onto
`tinyagents`, while the vendored crate now carries its own runtime under
[`vendor/tinyflows/crates/tinyflows/src/graph/`](../../../../vendor/tinyflows/crates/tinyflows/src/graph/)), and
[../cron/README.md](../cron/README.md) (`JobType::Flow`).

## Further reading

- [Parent module README](../../README.md)
- [tinyflows](../../../../vendor/tinyflows/README.md)
