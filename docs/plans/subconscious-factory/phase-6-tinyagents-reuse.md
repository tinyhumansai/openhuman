# Phase 6 — TinyAgents graph reuse & upstream changes

The subconscious runner (phase 1.2) is built on `tinyagents::graph` — the same
durable-workflow runtime the orchestration wake path uses. This phase tracks
(a) what we reuse as-is, and (b) the gaps that warrant changes **inside
`vendor/tinyagents`**, which is a git submodule of the separate repo
`tinyhumansai/tinyagents`: those changes go on their own branch there, raised
as their **own PR**, and the openhuman PR bumps the submodule pointer to the
merged commit (same workflow as `docs/plans/rlm-workflows/phase-2-tinyagents.md`).

## 6.1 Reused as-is (no upstream change)

| Need | tinyagents surface |
| --- | --- |
| tick topology + conditional quiet/changed routing | `GraphBuilder`, `Route`/command-routing, `START`/`END` |
| typed per-tick state | serde state + `LastValue` channels |
| crash-safe resume of an interrupted tick | `SqliteCheckpointer`, `CheckpointConfig`/`DurabilityMode` |
| injected behavior (profile = runtime) | the `dyn`-runtime node pattern from `orchestration/graph/build.rs` |
| stub-driven mechanics tests | `graph::testkit` |
| per-node timing / run snapshots | `graph::stream`, `graph::status` |
| bounded loops (none today, future kinds may loop) | `max_supersteps` backstop |

## 6.2 Candidate upstream changes (each = one small PR to `tinyhumansai/tinyagents`)

Confirm each gap against the vendored source before writing code; drop any
that turn out to already exist. Ordered by how much openhuman scaffolding
they delete:

1. **Wall-clock graph deadline.** Today `TICK_TIMEOUT` wraps the whole run in
   `tokio::time::timeout` from the outside, which cannot leave a clean
   checkpoint. Add an optional per-run deadline to `GraphExecution` (abort
   between supersteps + best-effort in-flight cancellation, surfaced as a
   typed `GraphTimeout` error with the last checkpoint intact). Benefits the
   orchestration wake graph too.
2. **External cancellation/supersede token.** The generation-counter
   supersede (`tick_generation`) currently only takes effect at the *end* of
   a tick (result discarded after the LLM spend). A shared cancel flag
   checked between supersteps — the graph twin of the REPL's `ReplCancelFlag`
   from the rlm-workflows plan — lets a newer tick abort an in-flight one
   before its reflect call completes. If the REPL flag work landed, reuse its
   primitive rather than adding a second one.
3. **Checkpoint GC / retention.** Ticks are frequent (every N minutes,
   forever), so `graph_checkpoints.db` grows unboundedly. Add a retention
   policy to `SqliteCheckpointer` (keep last K threads / prune completed
   threads older than T). The wake graph has the same latent issue.
4. **(Stretch) periodic-trigger helper.** A `graph::orchestration`-level
   "run this compiled graph every interval with jitter + overlap policy"
   would absorb our heartbeat fan-out (phase 4.3). Only worth it if the
   upstream maintainers want it — the openhuman heartbeat already works, so
   this is explicitly optional and last.

Items 1–3 are independent; none blocks phases 1–5. Phase 1 ships with the
outer `tokio::time::timeout` + end-of-tick supersede exactly as today, and
swaps to the upstream primitives when the submodule bump lands — each swap is
a small follow-up PR that deletes code.

## 6.3 Sequencing with the vendor submodule

1. Branch in `vendor/tinyagents` (e.g. `feat/graph-run-deadline`), implement
   with unit tests in the tinyagents style (`types.rs`/`mod.rs`/`test.rs`),
   PR against `tinyhumansai/tinyagents`.
2. Meanwhile openhuman phases 1–5 proceed against the current pin.
3. After the upstream merge: one openhuman PR per adopted primitive — bump
   the submodule pointer, replace the scaffolding (external timeout →
   deadline; end-of-tick supersede → cancel flag; add GC config), keep the
   behavior tests green.
4. Never point the openhuman submodule at an unmerged tinyagents branch on
   `main`-bound PRs.
