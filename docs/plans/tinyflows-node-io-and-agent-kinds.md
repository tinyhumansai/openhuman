# tinyflows — Node I/O Alignment & Selectable Agent Kinds (POA)

> **Status**: proposed · **Date**: 2026-07-05
> **Scope**: (A) fix the node input/output contract so producer/consumer shapes line up (agent ↔ tool*call ↔ merge), and (B) let a workflow pick \_which kind of agent* an `agent` node runs (coding agent, researcher, crypto agent, …) so each brings its own curated tool access.
> **Companion**: extends `docs/plans/tinyflows-integration/README.md`; audit source `vendor/tinyflows/docs/AUDIT-n8n-gap.md` (§3 bugs, §4b I/O alignment).
> **Reference product**: [n8n](https://github.com/n8n-io/n8n) — declared per-node output schemas, per-item execution, AI Agent node with pluggable tool/model sub-nodes.

---

## 0. Why this plan

Two problems surfaced in the audit that share a root cause — **nodes have no declared I/O contract**, so what one node emits and what the next reads are only accidentally aligned:

1. **Shape drift.** Every capability node wraps its host capability's raw return verbatim into `Item.json` (`agent.rs:115`, `tool_call.rs:31`, `http_request.rs:24`). The `agent` node emits three different shapes depending on sub-ports, and its plain shape flips between "parsed model JSON" and `{text}` at runtime (`caps.rs:359,369`). Downstream `=item.<field>` expressions therefore guess.
2. **No agent identity.** The `agent` node is a single bare `provider.chat` call with a free-form inline `tools` list (`caps.rs` `OpenHumanLlm::complete`, "no agent loop is driven here"). There is no way to say "run this step as the **coding** agent" or "as the **researcher**", even though OpenHuman already ships a 35-agent registry where each agent (`researcher`, `code_executor`, `crypto_agent`, …) declares its own toolset, model hint, sandbox, and iteration policy in an `agent.toml`.

Part A fixes the contract; Part B builds selectable agent kinds on top of the existing registry. They are sequenced so A1 (the agent envelope) lands before B (agent kinds emit into that same envelope).

---

## Part A — Node I/O alignment

### A0. Design principle: a normalized item envelope

Adopt a small, stable **output envelope** for capability nodes so every downstream expression has a guaranteed accessor, regardless of provider or config:

```jsonc
// agent / tool_call / http_request / code emit items shaped:
{
  "json":   <structured payload | null>,   // parsed/structured result when there is one
  "text":   <string | null>,               // human-readable text when there is one
  "raw":    <provider-native value>,        // escape hatch: the untouched capability return
  "error":  <null | { message, ... }>       // present only on continue/route error items
}
```

Rules: `=item.text` always resolves (or is explicitly `null`); `=item.json.<field>` is the structured path; `=item.raw` preserves today's behavior for anyone who needs the provider blob. This is additive — `raw` is exactly what nodes emit today — so migration is mechanical.

> Keep the crate **host-agnostic**: the envelope is defined in `vendor/tinyflows` (`src/data.rs` / node executors). The host adapters (`caps.rs`) already produce `{text}` / parsed JSON, so they map onto `json`/`text` directly.

### A1. Normalize the `agent` node output — _highest leverage, do first_

- **Crate** (`vendor/tinyflows/src/nodes/integration/agent.rs:115`): wrap the completion in the envelope instead of `Item::new(value)`. If `output_parser` ran, the coerced value goes in `json`; the completion text (when present) in `text`; the untouched response in `raw`; a model-elected tool result stays under `json.tool_result` **and** mirrors to a stable `tool_result` accessor (see A2).
- **Host** (`src/openhuman/tinyflows/caps.rs` `OpenHumanLlm::complete`): return `{ json: <parsed-or-null>, text: <response.text>, raw: <full response> }` rather than either the bare parsed object _or_ the `{text}` fallback. Removes the runtime shape-flip (audit M1).
- **Tests**: update `agent.rs` unit tests + `caps.rs` seam tests; add an e2e asserting `=item.text` resolves on both a JSON-emitting and a prose-emitting model (mock both).

### A2. Unify inline-tool vs `tool_call`-node result shape (audit M2)

Make a tool result reachable at the **same** path whether the tool ran inline in an `agent` node or as a standalone `tool_call` node. Standardize on the envelope: `tool_call` node emits `{ json: <tool output>, raw: <composio envelope> }`; the agent's inline tool result lands at `item.json.tool_result` and the node envelope's `raw` keeps the full completion. Document the one canonical accessor.

### A3. Per-item execution for integration nodes (audit M3 — the silent-drop trap)

Today `agent`/`tool_call`/`http_request` **always emit one item** and bind config against `input.first()` only, so `split_out (N) → tool_call` fires **once** and drops N−1 items.

- Add a node config flag `execution: "once" | "per_item"` (default `per_item` for `tool_call`/`http_request`; default `once` for `agent`, since an agent turn is usually batch-level — but allow `per_item`).
- In `per_item` mode: map the executor over `ctx.input`, re-resolving config per item (so `=item.x` means _this_ item), emit one output item per input, carry `paired_item` (`vendor/tinyflows/src/data.rs`).
- Touches `agent.rs`, `tool_call.rs`, `http_request.rs`, and the per-item resolution path in `nodes/mod.rs`.
- **Tests**: `split_out → tool_call` runs N times; `paired_item` lineage preserved; `once` mode unchanged.

### A4. Port-aware `collect_input` (audit M4 — untaken-branch leak + BUG-3/BUG-4)

`collect_input` (`vendor/tinyflows/src/engine.rs:171`) concatenates items from **every** predecessor slot regardless of `Edge.to_port` (stored, never read) or which port the predecessor emitted on. A node after a `condition` reads the not-taken branch's items.

- Read `Edge.to_port` / the predecessor's recorded `port`; collect only items the predecessor actually emitted on the connecting port.
- Enables **named merge inputs** (input A vs B) and removes the leak.
- Fold in the merge-barrier gap (BUG-4) and mixed-port fan-out drop (BUG-3) from the audit — same routing/lowering surface (`engine.rs:422`, `789-826`).
- **Tests**: condition-false slot not visible to a node wired on the true port; merge fed by a branching predecessor barriers correctly; the `main→a, main→b, error→h` shape runs both `a` and `b`.

### A5. `merge` modes (audit M5)

Add `merge.mode`: `append` (today's concat), `combine_by_key` (join items by a key field), `combine_by_position` (zip). Config-only change in `vendor/tinyflows/src/nodes/control_flow/merge.rs`; barrier semantics unchanged.

### A6. Author-time alignment lint (feeds Part C tooling)

A validation pass that, given a producer node's known/declared output envelope, flags downstream `=item.<field>` references the producer cannot emit — surfaced as structured, node-addressed diagnostics. Depends on tightening `validate.rs` (audit BUG-10). Wire into the agent-facing `validate` / `revise_workflow` tools so the builder agent gets the feedback.

### A-bugs (bundle with the above — from audit §3)

- **BUG-1 (security, hotfix now):** jq `env` builtin leaks host env — disable `jaq-std` default features / filter `env`/`input*` in `vendor/tinyflows/src/expr.rs:302`.
- **BUG-2:** `switch`/`transform` don't get the `nodes` scope — use `expr_scope` (`switch.rs:26`, `transform.rs:27`).
- **BUG-5/6:** sub-workflow HITL dropped; `on_run_finish` never fires on failure — verify against `FlowRunObserver`.
- **BUG-9:** `code`/`output_parser`/`sub_workflow` skip `=`-resolution — make expression binding uniform.

---

## Part B — Selectable agent kinds

### B0. The idea

Let an `agent` node declare **which agent** runs it, by referencing an OpenHuman registry agent:

```jsonc
{
  "kind": "agent",
  "config": {
    "agent_ref": "code_executor", // or "researcher", "crypto_agent", …
    "prompt": "=item.text",
    "connection_ref": "…", // still host-resolved, never model-supplied
    // optional per-node overrides:
    "model": "…",
    "max_iterations": 6,
    "tools_allow": ["grep", "edit"],
  },
}
```

When `agent_ref` is set, the host runs that **registered agent** — with _its_ curated toolset, model hint, sandbox mode, and iteration policy — as a full multi-turn agent loop, instead of the current single `provider.chat` call. A coding step gets coding tools; a research step gets `web_search`/`web_fetch`; a crypto step gets market tools. This is exactly the registry's existing contract (`src/openhuman/agent_registry/agents/*/agent.toml`).

### B1. Crate seam — new `AgentRunner` capability (host-agnostic)

The crate must not know about OpenHuman's registry, and "run a named agent to completion (multi-turn, tool-using)" is a **different capability** than `LlmProvider.complete` (single shot). Add a trait to `vendor/tinyflows/src/caps/mod.rs`:

```rust
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// Run a host-registered agent identified by `agent_ref` to completion.
    /// `request` carries prompt/input/overrides; `conn` is the opaque credential.
    async fn run_agent(&self, agent_ref: &str, request: Value, conn: Option<&str>) -> Result<Value>;
}
```

- Add `agent: Option<Arc<dyn AgentRunner>>` to `Capabilities` (optional so hosts without a registry keep working).
- **`agent.rs` dispatch**: if `config.agent_ref` is present _and_ `caps.agent` is wired → `run_agent(...)`; else fall back to today's `LlmProvider.complete` path. Both emit the A1 envelope, so downstream expressions are identical regardless.
- Mock impl in `caps/mock.rs` (echo `agent_ref` + request) so crate tests exercise both paths.
- `agent_ref` is **trusted config only**, never taken from model output (same rule as `tool_call.connection_ref`, `agent.rs:71-79`).

### B2. Host adapter — implement `AgentRunner` over the registry + delegate runtime

- `src/openhuman/tinyflows/caps.rs`: new `OpenHumanAgentRunner` implementing `AgentRunner`.
- `run_agent(agent_ref, request, conn)`:
  1. `agent_registry::ops::get_agent(agent_ref)` → resolve the entry (tools, model hint, sandbox, `max_iterations`, `iteration_policy`).
  2. Apply optional per-node overrides (`model`, `max_iterations`, and a **narrowing-only** `tools_allow` — a node may _subset_ the agent's tools, never add).
  3. Drive the existing delegate runtime (`src/openhuman/agent/tools/delegate.rs`) with a `TurnOrigin` of the flow run (`src/openhuman/agent/turn_origin.rs` already has a flow origin variant).
  4. Return `{ json, text, raw }` (A1 envelope): final structured output in `json`, final message text in `text`.
- **Autonomy/security**: the agent kind's tools are still gated by `SecurityPolicy` / autonomy tier; a `sandboxed` agent (e.g. `code_executor`) runs sandboxed; the flow's own approval gate still parks outbound actions. No new privilege path.
- **Depth/cost guard**: an agent node running a full agent loop inside a flow can fan out cost — bound it (reuse `MAX_SUB_WORKFLOW_DEPTH`-style counter or a per-run agent-invocation cap) and thread cancellation into the delegate run (today's sub-workflow path drops the token — BUG-5 — fix here too).

### B3. Authoring surface — ground the choice for the builder agent

- **Builder tool** `list_agent_profiles` in `src/openhuman/flows/builder_tools.rs`, backed by `agent_registry::ops::list_agents(false)`, returning `{ id, display_name, when_to_use, tools, sandbox_mode }`. Mirrors the existing `search_tool_catalog` pattern so the `workflow_builder` sub-agent picks a real `agent_ref` instead of hallucinating one.
- **Validation** (`vendor/tinyflows/src/validate.rs` + host): an `agent` node with an `agent_ref` that doesn't resolve is a structured validation error (needs the host to pass the known-agent set into validation, or validate host-side in `flows::ops::validate`).
- **workflow_builder prompt** (`src/openhuman/agent_registry/agents/workflow_builder/prompt.md`): document `agent_ref` and when to prefer a specialized agent over a bare completion.

### B4. UI — agent-kind picker

- The (upcoming) node config panel (`U2` in the integration plan) gets an **Agent kind** dropdown for `agent` nodes, populated from `agent_registry` `list_agents` RPC (already exists), showing `display_name` + `when_to_use`, with an optional tool-subset multiselect.
- Read-only canvas: show the chosen agent kind as a node badge.

---

## Part C — Sequencing & ownership

| Phase  | Work                                                                             | Depends on            | Surface                                           |
| ------ | -------------------------------------------------------------------------------- | --------------------- | ------------------------------------------------- |
| **C0** | BUG-1 env-leak hotfix; BUG-2 switch/transform scope                              | —                     | `vendor/tinyflows/src/expr.rs`, `control_flow/*`  |
| **C1** | A0 envelope + **A1 agent output normalization**                                  | C0                    | `agent.rs`, `caps.rs`                             |
| **C2** | A2 tool-shape unification; A3 per-item execution                                 | C1                    | `tool_call.rs`, `http_request.rs`, `nodes/mod.rs` |
| **C3** | A4 port-aware `collect_input` (+ BUG-3/4); A5 merge modes                        | — (parallel to C1/C2) | `engine.rs`, `merge.rs`                           |
| **C4** | **B1 `AgentRunner` capability** + mock                                           | C1 (shared envelope)  | `caps/mod.rs`, `caps/mock.rs`, `agent.rs`         |
| **C5** | **B2 host `OpenHumanAgentRunner`** over registry+delegate (+ BUG-5 cancellation) | C4                    | `tinyflows/caps.rs`, `agent/tools/delegate.rs`    |
| **C6** | B3 `list_agent_profiles` + `agent_ref` validation; A6 alignment lint             | C5, BUG-10            | `flows/builder_tools.rs`, `validate.rs`           |
| **C7** | B4 UI agent-kind picker                                                          | C5, U2 node panel     | `app/src/components/flows/*`                      |

Each phase ships with tests (crate unit + host seam + JSON-RPC E2E) per the repo's "tests before the next layer" rule, and records design decisions in `vendor/tinyflows/local/docs/11-decisions.md`.

## Open questions

1. **`agent_ref` in the crate**: prefer a dedicated `AgentRunner` capability (B1, recommended — keeps `LlmProvider.complete` a clean single-shot contract) vs. overloading `complete` to loop when it sees `agent_ref`. Recommendation: the capability.
2. **Per-node model/tool overrides**: allow narrowing only (`tools_allow` subsets the agent's toolset) — never widen, to preserve the registry's security envelope. Confirm this constraint.
3. **Cost/loop bounds** for agent-kind nodes inside a flow (and inside a sub-workflow): reuse the depth counter or introduce a per-run agent-invocation budget?
4. **Custom agents**: `upsert_custom_agent` already exists — user-defined agent kinds are addressable by `agent_ref` for free. Confirm they should be selectable in flows.
