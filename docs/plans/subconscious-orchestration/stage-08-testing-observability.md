# Stage 8 — End-to-end testing, observability & hardening

## Goal command

> Prove the whole loop with automated tests spanning both repos, audit logging/observability
> against the project's debug-logging rules, and harden failure modes (relay down, payment
> required, malformed envelopes, provider errors) so the layer can run unattended.

## Read first

- `tests/json_rpc_e2e.rs`, `scripts/test-rust-with-mock.sh`, `scripts/mock-api-core.mjs`.
- `gitbooks/developing/agent-observability.md`, `gitbooks/developing/testing-strategy.md`.
- `app/test/e2e/` — WDIO spec conventions.
- All stage docs in this folder (the invariants list in `README.md`).

## Deliverables

1. **Mock tiny.place relay** additions to the shared mock backend (`scripts/mock-api-core.mjs` or
   a sibling): DM send/receive + contacts endpoints (enforcing `403 not_a_contact` on unpaired
   DMs, crossing-request auto-accept) + a scriptable "wrapped session" that emits stage-1
   envelopes, so the full loop runs hermetically in CI.
2. **Cross-layer e2e** (`tests/json_rpc_e2e.rs` + mock relay): scripted session pairs first
   (stage-2 flow A and flow B both covered), then emits
   user+assistant envelopes → ingest → frontend pass 1 → reasoning cycle (mock provider) →
   frontend pass 2 → outbound DM captured by the mock → `orchestration.messages_list` shows the
   complete conversation; then a subconscious tick emits a directive and the next cycle carries it.
3. **Failure-mode suite**: relay 5xx/timeout during ingest and during reply (retry + no message
   loss), `payment_required` on send (surfaced, not retried blindly), malformed envelope flood
   (bounded log noise, Master fallback), provider error mid-graph (checkpoint resume, no duplicate
   outbound DM), scheduler_gate `Paused` (cycles defer, none dropped).
4. **Observability audit**: every node/stage logs entry/exit + correlation ids
   (`session_id`, `cycle_id`, `tick_id`); cycles and sub-agents visible in the agent-observability
   UI with usage/cost; `orchestration.status` exposes ingest-cursor lag and last-error. Add a
   `doctor` check for orchestration health if the doctor domain pattern fits.
5. **Frontend E2E** (WDIO, mock core): Brain → Orchestration tab renders pinned windows + a seeded
   session, live-updates on a pushed message, composer send round-trips.
6. **Docs**: update `gitbooks/developing/architecture/agent-harness.md` (new graph), a new
   `gitbooks/developing/architecture/orchestration.md` narrative, and
   `src/openhuman/about_app/` (user-facing feature registry). Run `pnpm docs:check`.

## Acceptance criteria

- One command each side proves the loop: `bash scripts/test-rust-with-mock.sh --test json_rpc_e2e`
  and `pnpm test` green, including the new suites; WDIO spec passes on the Linux lane.
- Coverage gate (≥80% changed lines) passes across frontend + rust-core lanes.
- Failure-mode suite green; no unbounded retry loops; no secret/body leakage in captured logs
  (assert with a log-scan test).
- Docs Drift lane green; about_app updated.
