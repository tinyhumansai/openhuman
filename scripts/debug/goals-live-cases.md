# `goals-live` test cases

Live exercises for the `memory_goals` domain via `scripts/debug/goals-live.mjs`.
Run against a running core (attached) or let the script spawn one.

> The list/add/edit/delete cases are pure RPC and need **no model**. The
> `reflect` case runs the turn-based `goals_agent` and therefore needs a
> configured provider/model + credentials in the target workspace.

## Prereqs

- A built core, or `--spawn-core` (which runs `cargo run --bin openhuman-core`).
- For `reflect`: a workspace with provider creds (use your real workspace —
  the default — not `--isolated-workspace`, which has none).

---

## Case 1 — CRUD only (no model needed)

Fastest smoke test of the list lifecycle and `MEMORY_GOALS.md` persistence.

```bash
pnpm debug goals-live --reset --case list --case add --case edit --case delete --case list-final
```

Expect: initial empty list → two goals added → first edited → last deleted →
final list shows one goal. Then `cat <workspace>/MEMORY_GOALS.md` to confirm
the on-disk markdown matches.

---

## Case 2 — Full flow against the running app

Uses your live core + real workspace, runs every case, and prints the
enrichment agent's reasoning + token/cost.

```bash
pnpm debug goals-live --show-thoughts
```

Expect: CRUD cases, then a `reflect` run with a `ran: true`, an agent summary,
the post-enrichment goals, a token/cost table, and the `goals_agent` thread
(its thoughts + `goals_list` / `goals_add` / … tool calls + tool results).

---

## Case 3 — Test a custom enrichment context (your prompt)

Feed the enrichment agent specific context and watch what it does. This is the
loop for iterating on `goals_agent/prompt.md`: edit the prompt, re-run, read
the thread.

```bash
pnpm debug goals-live --reset --case reflect --show-thoughts \
  --context "The user keeps asking about shipping the desktop app, wants a daily standup habit, and mentioned learning Rust deeply. Treat these as durable goals."
```

Expect: first-run bootstrap (empty list) → the agent populates initial goals
from the supplied context. The thread shows it calling `goals_list` first, then
`goals_add` per inferred goal.

---

## Case 4 — Spawned isolated core (CRUD determinism)

No external dependencies; spins a throwaway workspace and core. `reflect` will
no-op/fail without creds — scope to CRUD.

```bash
pnpm debug goals-live --spawn-core --isolated-workspace --keep-workspace \
  --case list --case add --case edit --case delete --case list-final
```

Expect: a fresh `MEMORY_GOALS.md` created under the kept temp workspace; the
path is printed at the end so you can inspect it.

---

## Reading the output

- **token usage / cost table** — per changed transcript session: input / output
  / cached input tokens, cache hit-rate, and charged USD (`charged_amount_usd`
  from the transcript `_meta`).
- **goals_agent thread** (`--show-thoughts`) — each non-system message: the
  assistant's content (thoughts + `<tool_call>` markers) and `tool` results.
  System prompts are hidden (length only) to keep output readable.
