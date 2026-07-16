# Flows: Agent-Friendliness Audit & Improvement Plan

**Date:** 2026-07-15 · **Scope:** how well the Workflows (Flows) product supports an AI agent
creating, editing, saving, testing, and debugging automations — across the Rust core
(`src/openhuman/flows/`, `tinyflows` engine seam), the agent tool belt
(`flows/tools.rs`, `flows/builder_tools.rs`), and the frontend canvas
(`app/src/pages/FlowCanvasPage.tsx` and friends).

---

## 1. Current architecture (baseline)

- A flow is a typed JSON node graph (`tinyflows::model::WorkflowGraph` — nodes, edges, 12
  `NodeKind`s, jq `=`-expressions for bindings), persisted in SQLite
  (`{workspace}/flows/flows.db`, `flow_definitions.graph_json`).
- 21 RPC controllers under `openhuman.flows_*` (`src/openhuman/flows/schemas.rs`).
- Agent surface: `propose_workflow` / `revise_workflow` (validate-only, never persist),
  `save_workflow` (update existing flows only), `dry_run_workflow` (mock capabilities),
  `run_flow` (saved flows, real effects), plus read tools (`list_flows`, `get_flow`,
  `get_flow_run`, `list_flow_connections`, `search_tool_catalog`, `get_tool_contract`,
  `get_tool_output_sample`, `list_agent_profiles`).
- Human-in-the-loop invariant is enforced **structurally**: the agent has no create tool;
  proposals render as `WorkflowProposalCard` / canvas diff previews, and only the user's
  explicit Save persists. `save_workflow` never touches `enabled` / `require_approval`.
- Frontend: React Flow canvas, explicit Save only (no autosave — a saved+enabled flow is
  live), client-side-only drafts (`/flows/draft` graph rides in router `location.state`),
  copilot panel driving `flows_build`.

Strengths worth preserving: the validate-gates return model-consumable, per-node,
"fix-and-retry" errors; the propose→accept→save loop gives real human oversight;
`get_tool_contract` / `get_tool_output_sample` are exactly the right shape of
machine-readable introspection.

---

## 2. Audit findings — where agents get hurt

### F1. Whole-graph blobs are the only edit unit (highest friction)
Every mutating/validating tool (`propose_workflow`, `revise_workflow`, `save_workflow`,
`dry_run_workflow`) requires the **entire** `WorkflowGraph` per call
(`flows/builder_tools.rs:114-122`, `:1712-1720`). There is no add-node / patch-node-config /
rewire-edge operation, at either the tool or RPC layer (`flows_update`'s `graph` param is
"Replacement WorkflowGraph", `schemas.rs:513`). For a 20-node flow, one config tweak means
re-emitting the whole graph — token-heavy, slow, and the #1 source of accidental
regressions (dropped nodes, mangled edges). `revise_workflow`'s "NOT a regeneration from
scratch" rule is advisory prose, not enforced.

### F2. The flow DSL has no queryable schema
The 12 node kinds, per-kind config shapes, port rules, and `.item` / `.item.json` envelope
semantics live only in prose: the 618-line `workflow_builder` prompt
(`flows/agents/workflow_builder/prompt.md`) and `propose_workflow`'s giant description
string (`flows/tools.rs:52-76`). `parameters_schema` describes node `config` as
"Kind-specific configuration; see tool description". Contrast with Composio actions, where
`get_tool_contract` returns real JSON schemas. Any agent outside the workflow_builder
persona (or a future model with a trimmed prompt) is flying blind, and prompt/code drift
is unchecked.

### F3. Validation iterates one error at a time, and paths diverge
`FlowValidation.errors` carries **at most one** error — tinyflows validation stops at the
first structural failure (`flows/types.rs:53-55`), so a graph with five problems costs
five round-trips. Several paths surface raw `.map_err(|e| e.to_string())` strings
(`ops.rs:89-92`, `:2204`). Additionally, the agent-tool save path layers extra hard gates
(binding resolvability, tool contracts, required-arg resolvability) that `flows_update`
itself doesn't run (`builder_tools.rs:1765-1770`) — so agent saves and UI saves are
validated differently, and a UI-saved flow can fail gates the agent is required to pass.

### F4. Capability asymmetries — RPC ops with no agent tool
| Operation | RPC | Agent tool | Consequence |
|---|---|---|---|
| Create flow | `flows_create` | none (by design) | OK as a safety choice, but there's no gated alternative for "create disabled draft" either |
| Enable/disable | `flows_set_enabled` | none | orchestrator prompt even references it; agent can only ask the user |
| Delete / duplicate | `flows_delete` / `flows_duplicate` | none | agent told to clone via get_flow→revise |
| List runs | `flows_list_runs` | none | agent has `get_flow_run` but must be handed a `run_id` externally — breaks the self-debug loop |
| Resume / cancel run | `flows_resume` / `flows_cancel_run` | none | agent can't progress a run paused on approval, or stop a runaway one |
| Standalone validate | `flows_validate` | none | validation only reachable bundled inside propose/revise/save |
| Import (n8n) | `flows_import` | none | agent can't help migrate automations |

### F5. No server-side draft; agent and UI can't share working state
Drafts exist only client-side: the `/flows/draft` graph lives in router `location.state`
(dropped on reload), and agent proposals live in the chat/canvas proposal card. There is
no durable draft the agent can iterate on across turns, no way for a chat-initiated build
and the canvas to reference the same in-progress graph by id, and a crash/reload loses
everything. This is also *why* F1 exists — with no server-side working copy, every tool
call must carry the full graph.

### F6. Last-write-wins everywhere; the UI is blind to agent edits
`update_flow_graph` has no version/etag/`updated_at` precondition (`store.rs:329-343`);
`Flow` has no version field. There are **no** socket/domain events for flow
create/update/delete/enable — only run-progress and approval events
(`useFlowRunProgress.ts`; `useFlowRunPoller.ts` notes "the flows engine emits no socket
events"). So: an agent `save_workflow` while the user has the canvas open is invisible,
and the user's next Save silently clobbers it (and vice versa). No revision history or
rollback exists, which is precisely what makes granting the agent more write power scary.

### F7. The agent cannot actually test what it built
`dry_run_workflow` runs against deterministic **mock** capabilities only — the prompt
spends ~200 lines (`prompt.md:540-605`) teaching the agent to reason around sandbox
artifacts. Real execution (`run_flow`) requires the flow to already be **saved** plus an
explicit user "yes" — a draft cannot be end-to-end tested at all. And the read-only
autonomy tier blocks even the side-effect-free mock dry-run
(`builder_tools.rs:1218-1229`), so a read-only agent cannot self-verify its own proposal.

### F8. "Workflow" means four different things
`flows/` (the product), the legacy `skills` tools literally named
`list_workflows`/`run_workflow`/`create_workflow` (SKILL.md system), `rhai_workflows` (an
in-turn scripting cell), and cron jobs. The orchestrator prompt has to explicitly warn
against crossing them (`agent_registry/agents/orchestrator/prompt.rs:105-110`), and
`run_flow` was named to dodge a collision with the legacy `run_workflow`. This taxes every
agent turn and every new contributor.

---

## 3. Improvement plan

Ordering principle: make edits cheap and correct first (F1–F3), then make shared state
durable (F5), then make concurrent writes safe + observable (F6) — because F6's
safety rails are the prerequisite for widening agent write capabilities (F4, F7).

### Phase 1 — Structured editing & introspection (F1, F2, F3)

1. **Graph patch operations.** New core op `flows::apply_graph_ops(base_graph, ops[]) ->
   Result<WorkflowGraph, StructuredErrors>` with ops like `add_node`, `update_node_config`
   (JSON-merge-patch on `config`), `rename_node`, `remove_node`, `add_edge`,
   `remove_edge`, `set_node_position`. Expose as a new agent tool `edit_workflow
   { flow_id | graph, ops[] }` that applies ops, runs the full validate+gate stack, and
   returns the updated graph + proposal payload (same contract as `revise_workflow`).
   Whole-graph tools stay for initial generation; iteration switches to ops.
2. **Queryable DSL schema.** Move per-node-kind config shapes into typed Rust schemas
   (source of truth), and expose `get_node_kind_contract { kind }` /
   `list_node_kinds` agent tools mirroring `get_tool_contract`. Generate the prompt.md
   node-kind section and `propose_workflow`'s description from the same source
   (docs-drift-style check) so prose can never diverge from code.
3. **Multi-error validation.** Change `FlowValidation.errors` to collect all
   structural errors (tinyflows `validate` change: accumulate instead of first-error),
   with structured entries `{node_id, field, code, message}`. Replace bare
   `.to_string()` error paths in `ops.rs` with the structured form.
4. **Unify validation planes.** Run the host hard gates (binding/contract/required-arg)
   inside `flows_update`/`flows_create` too — or, if UI permissiveness must stay,
   downgrade them to blocking-for-agents via an explicit `strict: bool` param rather
   than a divergent code path. Also give the agent a standalone `validate_workflow` tool
   (thin wrapper over `flows_validate` + gates) so it can check without proposing.

### Phase 2 — Core-managed local drafts (F5, and the real fix for F1's token cost)

5. **Draft entity — local JSON files, not a new table (for now).** Drafts are stored
   as plain JSON files on disk, managed by the core:
   `{workspace_dir}/flows/drafts/<draft-id>.json`, each holding
   `{id, flow_id?, name, graph, origin: chat|canvas|import, created_at, updated_at}`.
   No SQLite schema/migration. Same thin RPC surface on top:
   `flows_draft_create/get/update/list/delete/promote` — `promote` runs the existing
   create/update path (same gates, same forced `require_approval` floor) and removes
   the file. Key constraint this preserves: drafts must be readable/writable by **both**
   the agent tools (Rust core) and the canvas — which rules out frontend-only
   `localStorage`. File-based storage keeps drafts trivially inspectable and deletable,
   and can be migrated into a `flow_drafts` table later if drafts ever need querying,
   retention caps, or cross-device sync; the RPC contract stays identical either way.
6. **Agent tools on drafts.** `propose_workflow`/`revise_workflow`/`edit_workflow`/
   `dry_run_workflow` gain a `draft_id` mode: the graph lives in the core-managed
   draft file; tool calls
   carry only ops/instructions and get back diffs + validation. Cuts per-turn tokens
   dramatically and survives reloads/session hops.
7. **Frontend adoption.** `/flows/draft/:draftId` loads from core instead of
   `location.state`; import and proposal-accept create drafts; the unsaved-changes
   guard becomes "draft is saved, flow is not yet updated". Copilot and canvas now
   share one working copy by id.

### Phase 3 — Concurrency safety & observability (F6)

8. **Optimistic concurrency.** Add `version: i64` (or reuse `updated_at` as etag) to
   `Flow`; `flows_update` and `save_workflow` take `expected_version` and return a
   structured conflict error (with the current server graph) instead of clobbering.
   UI surfaces "flow changed since you opened it" with a reload/diff option.
9. **Flow mutation events.** Publish `DomainEvent::FlowChanged{flow_id, kind:
   created|updated|deleted|enabled_changed, actor}` on every mutation; bridge to a
   `flow:changed` socket event. FlowsPage refetches on it; FlowCanvasPage shows a
   banner when its open flow changes underneath (agent edits become visible in real
   time instead of silently).
10. **Revision history + rollback.** `flow_revisions` table capturing the prior
    `graph_json` on every update (capped, e.g. last 20), plus `flows_rollback` RPC and
    a `get_flow_history` agent tool. This is the safety rail that justifies Phase 4.

### Phase 4 — Widen agent capabilities behind existing gates (F4, F7)

11. **Debug loop tools:** `list_flow_runs { flow_id, limit }`, `resume_flow_run`
    (Execute + approval-gated), `cancel_flow_run` (Write). The agent can then find a
    failing run, diagnose it via `get_flow_run`, patch via `edit_workflow`, and verify.
12. **Gated create:** `create_workflow` agent tool → `flows_create` with hard-coded
    `enabled: false` + the existing forced `require_approval` floor,
    `PermissionLevel::Write` (approval gate). Enable/disable stays human-only
    (`flows_set_enabled` deliberately remains toolless). Add `duplicate_flow`
    (creates disabled copy) for the clone-then-edit pattern.
13. **Testability:** (a) allow `dry_run_workflow` on the read-only tier — it is
    mock-only and side-effect-free; (b) add a `test_run` mode that executes a draft
    with *real read-scope* capabilities only (generalizing `get_tool_output_sample`'s
    read-only-real-call precedent), refusing Write/Admin-class nodes; (c) let
    `run_flow` accept `draft_id` once drafts exist, still behind explicit user
    confirmation.

### Phase 5 — Builder UX: connector onboarding, tool discovery, chat separation

Today the builder assumes the user already knows their Composio landscape: the
`tool_call` config drawer makes them hand-type a toolkit slug before the per-toolkit
action dropdown appears (`app/src/components/flows/canvas/nodeConfig/composioFields.tsx`),
the connection selector only lists connections that already exist
(`flows_list_connections` → `nodeConfigFields.tsx:559`), and full-catalog search
(`search_tool_catalog` / `get_tool_contract`) is agent-tool-only with no RPC the UI can
call. When a graph needs an unconnected toolkit, the tool-contract gate just errors —
nothing walks the user through connecting it.

16. **Catalog RPCs for the UI.** Expose the existing agent-tool logic as controllers:
    `flows_search_tool_catalog { query, toolkit? }` and `flows_get_tool_contract
    { slug }` (thin wrappers over the same core code as `search_tool_catalog` /
    `get_tool_contract`, secret-free). One implementation, two consumers.
17. **In-canvas tool browser.** Replace the hand-typed toolkit slug with a searchable
    catalog picker in the `tool_call` config drawer (and a "browse tools" entry in the
    NodePalette): search across all toolkits, show description / required args /
    connected-state per result, select → fills toolkit+action and shows the contract's
    arg schema. Connected toolkits rank first; unconnected results carry a Connect
    badge.
18. **Required-connections surfacing.** Compute `required_connections:
    [{toolkit, connection_ref?, status: connected|missing}]` for any graph (derivable
    from the existing tool-contract gate) and include it in `flows_validate` output,
    the `workflow_proposal` payload, and `flows_get`. The proposal card and canvas
    validation banner render missing ones as explicit "Connect <toolkit>" CTAs
    deep-linking into the existing `/connections` connect flow, instead of a bare
    gate error. On return (connection created), re-validate automatically.
19. **Agent-side guidance.** Teach the workflow_builder prompt to treat a missing
    connection as a first-class outcome: propose the flow anyway, enumerate the
    required connections in the proposal summary, and tell the user which toolkits
    need connecting (the card's CTAs do the rest). Optionally add a read-only
    `list_connectable_toolkits` tool (catalog toolkits + connected flag) so the agent
    can steer toolkit choice toward what's already connected.
20. **Tag workflow chats separately from general chat.** Workflow-copilot
    conversations are ordinary core threads today, distinguishable only by a
    client-side `localStorage` mapping (`workflowCopilotThreads.ts` — user-scoped
    `copilot-thread:<flow>` keys). Instead, tag the thread server-side at creation:
    add a thread `kind` (e.g. `workflow_copilot`, with the associated
    `flow_id`/`draft_id` in thread metadata) in the `threads` domain, set it whenever
    `flows_build` / the copilot panel spawns a thread, and let thread-list queries
    filter by kind. The general `/chat` thread list excludes `workflow_copilot`
    threads; the flows UI lists a flow's builder threads from the core by tag. This
    also lets the copilot resolve "which thread belongs to this flow" server-side,
    demoting the fragile `localStorage` mapping to a cache (or deleting it).

### Phase 6 — Naming & prompt hygiene (F8)

21. Rename the legacy skills tools (`list_workflows`→`list_skills`,
    `run_workflow`→`run_skill`, etc., with deprecation aliases for one release) so
    "workflow" unambiguously means Flows in the agent's tool belt; delete the
    orchestrator-prompt disambiguation paragraph once done.
22. Shrink the workflow_builder prompt by replacing the node-kind reference and mock
    behavior table with pointers to the Phase 1 introspection tools (generated docs
    keep parity).

### Delivery: one PR, phased commits

All six phases ship together as **a single PR**, not as separate PRs per item. The
phases above define the *internal build order and commit structure* of that PR — each
numbered item lands as one or more focused commits, in phase order, so the branch is
reviewable commit-by-commit and bisectable — but the feature is reviewed, tested, and
merged as one unit. Rationale: the pieces are interdependent (drafts remove the need for
whole-graph tools, versioning/events are the safety rails that justify the wider tool
belt, the prompt shrink depends on the introspection tools existing), and shipping them
piecemeal would leave the agent surface in inconsistent intermediate states across
releases.

| Phase | Depends on | Rough size | Risk |
|---|---|---|---|
| 1 (patch ops, schema tools, multi-error) | — | M–L (tinyflows crate change for multi-error) | Low |
| 2 (core-managed local drafts) | 1 helps | S–M | Low — additive files/RPC, no DB migration |
| 3 (versioning, events, history) | — (parallel to 2) | M | Medium — touches UI save path |
| 4 (new agent tools) | 3 (safety rails) | S–M each | Medium — permission review each |
| 5 (connector onboarding, tool browser, chat tagging) | 1 (contract gate reuse) | M | Low–Medium — UI + additive RPCs + thread `kind` |
| 6 (renames) | — | S | Low (needs deprecation window) |

Build within the branch in phase order (1 → 2 → 3 → 4 → 5 → 6): items 1–3 (Phase 1) deliver
the biggest agent-experience win and require no changes to the human-in-the-loop model;
Phase 3's rails must be in place before Phase 4's write tools are enabled. The PR
description should map commits to plan items so reviewers can follow the same structure.

---

## 4. Non-goals / explicitly preserved invariants

- **No autosave** on the canvas — a saved+enabled flow is live; the explicit Save gate stays.
- **Enable/disable remains human-only**; agent-created flows are born disabled.
- The forced `require_approval = true` floor for side-effect graphs stays uncloseable
  by the agent.
- `propose → user accepts → save` remains the default chat flow; new write tools are
  additive and approval-gated, not a replacement for proposals.
