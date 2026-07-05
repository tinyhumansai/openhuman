# Phase 5 — Test matrix, docs, rollout

## 5.1 Test matrix (cumulative — most land inside phases 1–4)

| Layer | Test | Phase |
| --- | --- | --- |
| runner | FakeProfile: quiet / fail / rate-cap-halt / superseded lifecycles | 1 |
| store | key namespacing + legacy→`memory:` migration round-trip | 1 |
| memory profile | observe over seeded diff store; origin escalation; mode→iteration caps | 2 |
| ported | all of today's `engine_tests.rs` against `SubconsciousInstance<MemoryProfile>` | 2 |
| tinyplace profile | observe/reflect/commit against scripted provider; idle-NONE advances cursor | 3 |
| cross-instance | two instances, one workspace: independent state keys, halts, no lock coupling | 3 |
| factory | `enabled_kinds` gating per config; `make_subconscious` per kind | 4 |
| registry | bootstrap set, user-switch reset clears map, deprecated alias | 4 |
| heartbeat | fan-out ticks only elapsed cadences; concurrent tick spawn | 4 |
| RPC | `status` legacy fields mirror memory; `instances` rows; `trigger kind=…` | 4 |
| json_rpc_e2e | extend `tests/json_rpc_e2e.rs`: `subconscious.status` shape incl. `instances` | 4 |
| isolation | tinyplace source-scan (no agent/toolset imports); agent.toml scan (exists) | 3 |

Coverage gate: each phase is its own PR-sized slice with ≥80% diff coverage;
the extraction phases (2, 3) inherit most coverage from ported tests.

## 5.2 Docs

- `src/openhuman/subconscious/README.md` — rewrite around the factory: the
  generic tick skeleton, the profile table (memory / tinyplace), per-instance
  persistence keys, RPC additions. Keep the gotchas list (post-login
  bootstrap, status-never-locks, state-advances-on-success, quiet-tick
  short-circuit, taint) — all still true, now per instance.
- `orchestration/mod.rs` module docs — stage 6 wording: the review is now
  driven by the tinyplace subconscious instance, not inlined in the memory
  tick.
- `gitbooks/developing/architecture/agent-harness.md` — if it names the
  subconscious loop, reflect the split. Run `pnpm docs:generate` +
  `pnpm docs:check` (Docs Drift lane) in the final slice.
- `about_app` copy (phase 4.5).

## 5.3 Rollout & risk

- **No data-shape risk**: the only migration is KV key renaming inside
  `subconscious.db` (phase 1.3), designed to be old-version-tolerant. The
  orchestration store is untouched.
- **Behavioral deltas to call out in the PR series** (all intentional):
  1. The orchestration review no longer piggybacks on the memory tick — it
     runs on its own cadence, so it also runs when the memory world is quiet
     (today a memory-provider outage silently starves steering; after phase 3
     it doesn't).
  2. A rate-cap halt on one world no longer pauses the other.
  3. `subconscious.status` gains fields (additive only).
- **Branch/PR sequencing**: one branch per phase off `upstream/main`
  (`feat/subconscious-profile-core`, `feat/subconscious-memory-profile`, …),
  stacked; each independently green on ci-lite. Phase 2+3 could merge as one
  PR if review prefers seeing the shim appear and disappear together.

## 5.4 Explicit non-goals (this plan)

- No change to the orchestration wake graph, compression ratio, context-guard
  thresholds, or steering contract — those already implement the spec.
- No new subconscious kinds beyond the two (the factory makes them cheap
  later; e.g. a `team` world or a `channels` world).
- No frontend redesign beyond phase 7's scoped additions (instance cards +
  steering header) — no new pages, routes, or Redux slices.
- The opt-in event-driven trigger pipeline (`subconscious_triggers` +
  `LongLivedSession`) keeps its current shape; folding it into a profile is a
  possible follow-up once the factory exists.
