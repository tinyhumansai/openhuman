# Phase 6 — Tests (written last)

Per the feature brief, tests are back-filled **after** the behavior lands
(phases 3–5). Target: ≥80% changed-line coverage (the `pr-ci.yml` diff
gate) for the openhuman side. TinyAgents-side tests ship inside Phase 2
(that crate's convention does not defer tests).

## 6.1 Unit tests — `src/openhuman/rlm/`

Use tinyagents testkit stubs (`ScriptedModel`, `FakeTool`, `StubAgent`) so
no network and no real provider is needed.

`policy.rs` (`#[cfg(test)] mod tests`):
- timeout clamped to [1, 3600] and to the global cap; limits override
  ceiling (≤2× default) enforced; readonly tier → policy builder refuses.

`bridge.rs`:
- excluded tools (`rlm`, `spawn_*`, `run_workflow`) absent from registry;
- an openhuman fake tool is callable via the registry and returns its
  `ToolResult` content;
- subagent capability maps `agent_query` prompt → `run_subagent` input
  (with a stubbed runner seam) and threads depth.

`sessions.rs`:
- namespace persists across two `eval_cell`s in one session;
- distinct `session_id`s isolated; thread-scoped keys isolated;
- LRU eviction + idle TTL; `close_session` drops the entry;
- concurrent second call on a busy session → typed "busy" error (no
  deadlock);
- poisoned/errored session dropped, not reused.

`ops.rs` / `tools.rs` (the error-proofing matrix, one test per row of the
Phase 5 taxonomy):
- happy path: script calling `tool_call` + `answer` → RlmEvalResponse with
  stdout/value/final_answer;
- parse error → `is_error` tool result containing the rhai diagnostic;
- `while true {}` → Timeout within policy bound (bounded test time: 1–2 s
  policy);
- hanging fake tool future → Timeout via bridge race;
- `max_tool_calls = 1` + two calls → LimitExceeded naming the limit;
- unknown tool name → error listing registered names;
- cancel flag fired mid-cell → Cancelled, session still usable after;
- oversized output → LimitExceeded; result truncation applied;
- schema validation: missing `script`, bad `timeout_secs` bounds.

`RlmTool` metadata tests: name/permission/scope/timeout_policy/display
label — the same shape other domains' tool tests use.

## 6.2 Integration — `tests/` (mock backend)

Extend `tests/json_rpc_e2e.rs` / `scripts/test-rust-with-mock.sh` **only if**
the tool is reachable over RPC in v1; since `scope() = AgentOnly`, v1
instead adds a Rust integration test that assembles the turn harness with
the RLM tool registered and drives a scripted turn where the model calls
`rlm` with a fan-out script (mock provider supplies the tool_call). Asserts:
progress events observed on the `AgentProgress` sink, final tool result
well-formed.

## 6.3 Frontend

No new UI surface in v1 (progress rides existing tool-call timeline cards),
so no Vitest additions beyond any i18n keys if a display string is added —
if one is, add the key to `en.ts` + all locale files and run
`pnpm i18n:check`.

## 6.4 Commands

```bash
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml
pnpm debug rust rlm                       # targeted domain tests
bash scripts/test-rust-with-mock.sh       # full rust suite
pnpm typecheck && pnpm lint               # app unchanged, still gate
cd vendor/tinyagents && cargo test --features repl
```
