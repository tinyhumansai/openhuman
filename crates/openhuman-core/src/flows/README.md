# Flows

Saved automation workflows — the graphs a user builds on the canvas or the
copilot builds for them. Owns CRUD/enable/run/resume/cancel for saved flows,
the trigger → run bridge, the authoring tools (propose/create/edit/validate/
dry-run/save), discovery/suggestion tools, and the medulla workflow-plane
bridge. Does NOT own the workflow engine itself: `tinyflows` (vendored) owns
the model, validation, compilation, and its own in-crate state-graph runtime,
and reaches OpenHuman only through the capability traits `tinyflows/` here
implements.

See [gitbooks/developing/architecture/flows-on-tinyagents.md](../../../../gitbooks/developing/architecture/flows-on-tinyagents.md)
for the run pipeline and security model (note: it still describes `tinyflows`
as lowering onto `tinyagents`; the vendored crate now carries that runtime
in-crate under `vendor/tinyflows/crates/tinyflows/src/graph/`), and
[`tinyflows/README.md`](tinyflows/README.md) for the capability seam; this
file is the directory map.

## Gate shape — leaf, not facade

The whole family (`flows` + `flows::tinyflows`) is gated at `pub mod flows;`
in `crates/openhuman-core/src/lib.rs` behind `#[cfg(feature = "flows")]`, and
every submodule inherits that gate. There is deliberately **no `stub.rs`**:
every symbol reached from outside is a registration site —
`core::all::all_flows_registered_controllers`, `core::jsonrpc`'s
`FlowTriggerSubscriber`, `core::runtime::services`' boot reconcile
(`sweep_orphaned_running_runs_on_boot`, `reconcile_schedule_triggers_on_boot`),
`medulla_bridge::install`, the agent-tool `vec!` in `tools::ops`, and the
`workflow_builder` / `flow_discovery` entries in
`agent::registry::agents::loader::BUILTINS`. A
registration site wants *absence* when the feature is off, not a
disabled-error stub, or `flows.*` becomes a known method that fails at
runtime. See `voice/` for the facade+stub shape used when a domain is called
from always-compiled code.

## Public surface

- `pub mod ops` (split into submodules under `ops/`, e.g. `ops/execution.rs`, `ops/resume.rs`, `ops/builder.rs`, `ops/triggers.rs`, `ops/drafts.rs`, `ops/discovery.rs`, `ops/wiring_warnings.rs`, `ops/inference_readiness.rs`) — CRUD (`flows_create/get/list/update/delete/duplicate/import/validate`), revision history (`flows_get_history`, `flows_rollback`), run lifecycle (`flows_run`, `flows_run_detached`, `flows_resume`, `flows_cancel_run`, `flows_list_runs`, `flows_list_all_runs`, `flows_get_run`, `flows_prune_runs`), `flows_set_enabled` (only `schedule` triggers need an enable-time binding — `cron::add_flow_schedule_job` / `cron::remove_job`; `app_event` flows are matched at dispatch time against enabled flows, and `webhook` binding is logged as not implemented), builder (`flows_build`, `flows_build_cancel`, `flows_search_tool_catalog`, `flows_get_tool_contract`, `flows_list_connections`, `flows_required_connections`, `flows_approval_manifest`), discovery (`flows_discover`, `flows_list_suggestions`, `flows_dismiss_suggestion`, `flows_mark_suggestion_built`), drafts (`flows_draft_create/get/update/list/delete/promote`), and boot/periodic reconciliation (`sweep_orphaned_running_runs_on_boot`, `reconcile_schedule_triggers_on_boot`, `sweep_expired_parked_runs`).
- `pub mod bus` (split into `bus/trigger.rs`, `bus/run_digest.rs`, `bus/dedup_commit.rs`) — three subscribers, all constructed in `core/jsonrpc.rs`: `FlowTriggerSubscriber` (the trigger → run bridge: `DomainEvent::FlowScheduleTick` and `ComposioTriggerReceived` are matched against enabled flows' trigger nodes and spawn `ops::flows_run`; `WebhookIncomingRequest` is observed and logged only — webhook dispatch is not implemented), `FlowRunDigestSubscriber` (on a successful `FlowRunFinished`, writes a run digest into the flow's private memory namespace), and `DedupCommitSubscriber` (on `FlowRunFinished`, commits or rolls back every `dedup` node's tentative key set). `extract_trigger_kind` / `extract_trigger_config` are reused by `ops` to decide what `flows_set_enabled` and `flows_update` must bind or rebind.
- `pub mod medulla_bridge` — backs the medulla harness protocol's workflow plane (`platform::socket::medulla::workflows::WorkflowBridge`) with this store: projects saved `Flow`s onto the wire `WorkflowDescriptor`, serves the three read RPCs, and runs a `workflow_builder` copilot turn with host-enforced approval guards (creates always `require_approval: true`, updates never lower it, automatic-trigger creates are saved disabled).
- `pub mod catalogue` — lists saved flows as `Workflow` entries with `WorkflowScope::Flow` in the shared skill catalogue, so `skill_search` sees one list instead of skills and flows separately.
- `pub mod node_contracts` — host overlay on `tinyflows::catalog`'s node-kind contracts: attaches host-specific facts (which `tool_call` slugs resolve to Composio vs. native `oh:` tools, which trigger kinds actually dispatch here) without touching the portable contracts. Re-exports `all_node_kind_contracts`, `node_kind_contract`, `NODE_KINDS`, `ConfigField`, `PortSpec`, `NodeKindContract`.
- `mod store` / `mod draft_store` (private) — bind `tinyflows_sqlite::flows` / `tinyflows_sqlite::drafts` to `<workspace_dir>/flows`; `kv_get`, `kv_set`, and `upsert_flow_run_step` are re-exported from `store` for the `tinyflows::caps::FlowStateStore` seam and the run observer.
- `mod schemas` (private; `all_flows_controller_schemas` / `all_flows_registered_controllers` re-exported) — RPC/CLI controller surface under the `flows` namespace, 36 functions. The `ControllerSchema` lookups are split into `schemas/definition_schemas.rs` (`create`, `duplicate`, `validate`, `import`, `get`, `list`, `list_connections`, `update`, `delete`, `set_enabled`, `get_history`, `rollback`, `required_connections`, `approval_manifest`), `schemas/run_schemas.rs` (`run`, `run_detached`, `resume`, `cancel_run`, `list_runs`, `list_all_runs`, `get_run`, `prune_runs`), `schemas/builder_schemas.rs` (`build`, `build_cancel`, `search_tool_catalog`, `get_tool_contract`, `discover`, `list_suggestions`, `dismiss_suggestion`, `mark_suggestion_built`), and `schemas/draft_schemas.rs` (`draft_create/get/update/list/delete/promote`); all 36 `handle_*` thin handlers live in `schemas_handlers.rs`.
- `pub mod tools` — `ProposeWorkflowTool`, `RunFlowTool`. All 27 tool structs below are re-exported by glob from `crates/openhuman-core/src/tools/mod.rs` and pushed onto the agent tool list in `tools/ops.rs` under `#[cfg(feature = "flows")]`.
- `pub mod builder_tools` (split into submodules under `builder_tools/`: `draft_revise.rs`, `draft_edit.rs`, `draft_validate.rs`, `flow_reads.rs`, `run_control.rs`, `persistence.rs`, `connection_reads.rs`, `catalog_search.rs`, `tool_contract.rs`, `kind_reads.rs`, `dry_run.rs`, `dry_run_diagnostics.rs`) — the authoring toolset: `ReviseWorkflowTool`, `EditWorkflowTool`, `ValidateWorkflowTool`, `GetFlowHistoryTool`, `ListFlowRunsTool`, `ResumeFlowRunTool`, `CancelFlowRunTool`, `CreateWorkflowTool`, `DuplicateFlowTool`, `ListConnectableToolkitsTool`, `ListFlowsTool`, `GetFlowTool`, `GetFlowRunTool`, `ListFlowConnectionsTool`, `SearchToolCatalogTool`, `GetToolContractTool`, `GetToolOutputSampleTool`, `ListAgentDefinitionsTool`, `ListNodeKindsTool`, `GetNodeKindContractTool`, `DryRunWorkflowTool`, `SaveWorkflowTool`.
- `pub mod discovery_tools` — `SuggestWorkflowsTool`.
- `pub mod memory_tools` — `FlowMemoryRecallTool`, `FlowMemoryRememberTool`, plus `flow_namespace` / `FLOW_MEMORY_NAMESPACE_PREFIX` / `cross_flow_recall` (re-exported from `mod.rs` because the tinyflows `memory` node's `OpenHumanMemory` adapter needs byte-identical `scope: "flows"` results).
- `pub mod agents` — first-class built-in sub-agents: `workflow_builder` (authoring copilot) and `flow_discovery` (read-only suggestion scout); their `agent.toml` and `prompt::build` are referenced by path from the `BUILTINS` slice in `agent/registry/agents/loader.rs`.
- `pub mod skills` (needs both `flows` and `skills` features) — registers the portable `tinyflows-copilot` `flow-authoring` manual with OpenHuman's native skill runtime.
- `pub mod tinyflows` — the capability seam (`caps/`) implementing `tinyflows`'s traits over real OpenHuman services, plus `observability.rs` (`FlowRunObserver`), `memory_adapter.rs` (`OpenHumanMemory`), and `langfuse_export.rs`. Has its own [README](tinyflows/README.md).
- Re-exported model types (from `tinyflows_catalog`, not owned here): `Flow`, `FlowConnection`, `FlowDraft`, `FlowImport`, `FlowRevision`, `FlowRun`, `FlowRunStep`, `FlowRunTrigger`, `FlowSuggestion`, `FlowValidation`, `FlowValidationError`, `SuggestionStatus`, `DraftOrigin`, plus `types`, `run_registry`, `build_registry`, and `n8n_import` (the format importer).

## Calls into

- `vendor/tinyflows/` — the actual workflow model, validation, compilation, and run engine; this domain never re-implements it.
- `crates/openhuman-core/src/agent/tinyagents/` — message/tool-call/usage conversions used by the `llm` and `prompt` capabilities; `agent` nodes pass an explicit run context into nested harness turns through the `agent` capability (`tinyflows/caps/agent.rs`).
- `crates/openhuman-core/src/cron/` — `add_flow_schedule_job` arms a schedule-triggered flow as a `JobType::Flow` cron job; the scheduler fires it by publishing `DomainEvent::FlowScheduleTick`, which `bus::FlowTriggerSubscriber` picks up.
- `crates/openhuman-core/src/platform/socket/medulla/workflows.rs` — `WorkflowBridge` trait implemented by `medulla_bridge`.
- `crates/openhuman-core/src/skills/` — the `Workflow` / `WorkflowScope` catalogue types used by `catalogue.rs`, and the native `BundledSkill` mechanism that exposes the portable `tinyflows-copilot` authoring manual.
- `crates/openhuman-core/src/memory/` — `memory_tools`/`tinyflows::memory_adapter` read/write agent memory under the `flows` scope.

## Called by

- `crates/openhuman-core/src/core/all.rs` — registers `all_flows_registered_controllers()` under `#[cfg(feature = "flows")]`.
- `crates/openhuman-core/src/core/jsonrpc.rs` — constructs and subscribes `FlowTriggerSubscriber`, `FlowRunDigestSubscriber`, and `DedupCommitSubscriber` at startup.
- `crates/openhuman-core/src/core/runtime/services.rs` — runs `sweep_orphaned_running_runs_on_boot` and `reconcile_schedule_triggers_on_boot` during core boot and calls `medulla_bridge::install`; `platform/socket/ops.rs` installs the bridge on the socket path as well.
- `crates/openhuman-core/src/tools/ops.rs` — pushes all 27 flows tools onto the agent tool list (`tools/mod.rs` re-exports the four tool modules).
- `crates/openhuman-core/src/agent/registry/agents/loader.rs` — registers `workflow_builder` and `flow_discovery` as built-in archetypes.
- `crates/openhuman-core/src/agent/session_host/` (`builder/factory.rs`, `turn/tools.rs`) — extends the skill catalogue with `catalogue::flow_entries`.

## Tests

- Unit: `*_tests.rs` attached with `#[path]` to nearly every top-level file (`ops_tests*`, `bus_tests*`, `builder_tools_tests*`, `catalogue_tests.rs`, `discovery_tools_tests.rs`, `medulla_bridge_tests.rs`, `memory_tools_tests.rs`, `node_contracts_tests.rs`, `schemas_tests.rs`, `tools_tests.rs`), plus `import_tests.rs` for the n8n importer (declared in `mod.rs`).
- `tinyflows/` has its own suite: `checkpoint_compat_tests.rs`, `memory_adapter_tests.rs`, `memory_node_e2e_tests.rs`, `observability_tests.rs`, `langfuse_export_tests.rs`, `tinyflows_tests.rs`, and `caps/*_tests.rs`.
- Not compiled: `types.rs` / `types_tests.rs` are declared by no module (`flows::types` resolves to `tinyflows_catalog::types`, and `store.rs` has no test attachment). They are leftovers from moving the model and store into `tinyflows-catalog` / `tinyflows-sqlite`.

## Related docs

- [gitbooks/developing/architecture/flows-on-tinyagents.md](../../../../gitbooks/developing/architecture/flows-on-tinyagents.md) — the run pipeline, capability seam, and two-layer security model.
- [`../cron/README.md`](../cron/README.md) — `JobType::Flow` and the schedule-trigger binding.
