# Stage 7 — RPC surface + TinyPlaceOrchestrationTab wiring

## Goal command

> Expose the orchestration layer over JSON-RPC (`openhuman.orchestration_*`) and rewire
> **`app/src/components/intelligence/TinyPlaceOrchestrationTab.tsx`** onto it: the tab's
> master / subconscious / per-session chat windows come from the stage-3 store's real
> classification and metadata instead of the current `chatKindForEnvelope` string heuristics; add
> live updates over the core socket and a composer for the Master window (owner → agent DM).

## Read first

- `app/src/components/intelligence/TinyPlaceOrchestrationTab.tsx` (+ its test) — current shape:
  `ChatKind`/`ChatWindow` model, pinned master+subconscious, `LoadState` incl. `payment_required`.
- `app/src/lib/agentworld/invokeApiClient.ts` — `callCoreRpc` pattern, `PaymentRequiredError`.
- `src/openhuman/tinyplace/schemas.rs` — internal-registry controller pattern to copy.
- `src/openhuman/orchestration/{types.rs, store.rs, ops.rs}` (stages 3–6).
- Socket push: `app/src/services/socketService.ts`, `src/openhuman/socket/` — how domain events
  reach the renderer (dual-socket sync rule).
- i18n: `app/src/lib/i18n/en.ts` `tinyplaceOrchestration.*` block (exists; extend).

## Deliverables

1. **RPC controllers** (`orchestration/schemas.rs`, internal registry — renderer-only, not
   agent-advertised):
   - `orchestration.sessions_list` → `{ sessions: OrchestrationSession[] }` (incl. computed
     `active`, unread counts).
   - `orchestration.messages_list { chat: "master"|"subconscious"|{sessionId}, limit, before? }`.
   - `orchestration.send_master_message { body }` → DM to the agent's Master counterpart via the
     signal-send op (this is the human steering the front-end agent).
   - `orchestration.mark_read { chat }`.
   - `orchestration.status` → steering directive (current), last tick, ingest cursor health.
2. **Renderer client** `app/src/lib/orchestration/orchestrationClient.ts` following the
   `invokeApiClient` conventions (`callCoreRpc('openhuman.orchestration_…')`, typed results,
   `PaymentRequiredError` passthrough where relevant).
3. **Tab rewire** (keep the existing layout/UX and testids):
   - Data source: `sessions_list` + per-selected-chat `messages_list` (drop the client-side
     bucketing of raw envelopes; keep the file's `ChatWindow` view model).
   - Live updates: subscribe to an `orchestration.message` socket event (bridge
     `DomainEvent::OrchestrationSessionMessage` / `ReplyReady` / steering-emitted through the
     socket domain) → targeted refetch of the affected chat; keep manual Refresh.
   - Master composer: input + send via `send_master_message`, optimistic append, error surface.
   - Unread: `mark_read` on chat open; badge from server counts.
   - Preserve `payment_required` and error states; loading per-pane rather than whole-tab.
4. **i18n**: new keys (composer placeholder, send, steering banner, read errors) added to `en.ts`
   **and all 13 other locales** (`pnpm i18n:check`, `pnpm i18n:english:check` clean).
5. **Steering visibility**: pinned Subconscious window shows directives (stage 6 feed); header
   chip on the tab surfaces the current directive from `orchestration.status`.

## Tasks

1. Controllers + schemas + `all.rs` wiring; unit tests on handlers (`RpcOutcome` paths, bad chat
   key, empty stores).
2. Socket bridge for the three orchestration events (follow an existing domain's socket relay).
3. `orchestrationClient.ts` + Vitest (RPC name mapping, error classification).
4. Tab refactor: extract data hooks (`useOrchestrationChats`) so the component stays presentational
   and under the ~500-line rule; update `TinyPlaceOrchestrationTab.test.tsx` to mock the new
   client; add tests for composer send, live-update refetch, unread clearing.
5. i18n additions across locales.
6. `tests/json_rpc_e2e.rs`: seed store → `sessions_list`/`messages_list`/`send_master_message`
   round-trip over real RPC with the mock backend.

## Acceptance criteria

- With stages 1–6 running and a wrapped Codex/Claude session active, the tab shows that session's
  window with real label/workspace metadata and both user and assistant messages, updating live
  without manual refresh.
- Master composer sends an E2E DM that reaches the front-end agent (verified in e2e via mock).
- Subconscious window shows emitted steering directives.
- `pnpm typecheck`, `pnpm lint`, `pnpm test`, `pnpm i18n:check` green; no `import.meta.env` reads;
  no dynamic imports; all RPC through `core_rpc_relay`.
