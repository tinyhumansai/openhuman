# Phase 7 — UI: see and interact with both subconscious kinds

Goal: the two instances become visible, distinguishable, and individually
triggerable in the app — in the **Subconscious tab** (both kinds' health and
controls) and in the **TinyPlace Orchestration tab** (the tinyplace kind in
its natural habitat, next to the steering it produces). Depends on the
phase-4 RPC additions (`instances[]` on status, `kind` on trigger); pure
`app/src` work otherwise. Existing building blocks are reused — notably the
orchestration tab already renders the pinned Subconscious chat window
(`chat.kind === 'subconscious'` in `TinyPlaceOrchestrationTab.tsx`), which is
where directives stream in today via `useOrchestrationChats`.

## 7.1 Types + clients (`app/src/utils/tauriCommands/subconscious.ts`)

- Extend `SubconsciousStatus` with the additive phase-4 fields:
  `instances: SubconsciousInstanceStatus[]`, where each row is today's status
  shape plus `instance: 'memory' | 'tinyplace'`. Legacy top-level fields keep
  mirroring the memory instance, so nothing breaks while the UI migrates.
- `subconsciousTrigger(kind?: 'memory' | 'tinyplace' | 'all')` — optional
  param passed through to `openhuman.subconscious_trigger`; no-arg call keeps
  today's behavior (memory).
- All calls stay on `callCoreRpc` / `core_rpc_relay` per the frontend rules.

## 7.2 Hook (`app/src/hooks/useSubconscious.ts`)

- Surface `instances` alongside the existing fields; add
  `triggerTick(kind?)` and per-kind `triggering` state (two buttons must not
  share one spinner).
- Keep polling/refresh cadence as-is; one status call still feeds everything.

## 7.3 Subconscious tab (`IntelligenceSubconsciousTab.tsx`)

Rework the status area from "one engine" to **two instance cards** under the
existing mode/interval controls (which continue to govern the memory
instance — mode semantics don't apply to tinyplace):

- **Memory card** ("Your world" / connected sources): today's status row —
  total ticks, last tick, consecutive failures, provider-unavailable banner
  with the Settings deep-link — plus its own *Run now*.
- **TinyPlace card** ("Orchestration steering"): enabled state (mirrors
  `orchestration.enabled`; show a disabled-with-hint state when off), last
  review tick, failures/halt reason, *Run review now*
  (`triggerTick('tinyplace')`), and a "View directives →" link that navigates
  to the TinyPlace Orchestration tab with the Subconscious window selected.
- Shared `SubconsciousInstanceCard` component
  (`app/src/components/intelligence/SubconsciousInstanceCard.tsx`) so a third
  kind later is a data change, not new JSX.

## 7.4 TinyPlace Orchestration tab (`TinyPlaceOrchestrationTab.tsx`)

The pinned Subconscious chat window already shows emitted directives (built
in an earlier slice — reuse, don't rebuild). Add the *instance* dimension on
top of it:

- A compact **steering status header** above that window's transcript:
  current directive (or "no active directive"), `expires_after_cycles`
  countdown against the live cycle counter, last review time, and a
  *Run review now* button — same `triggerTick('tinyplace')` path as 7.3.
  Data comes from the phase-4 status row plus a small read RPC if needed
  (`orchestration.status` already exists as the stage-7 read surface; extend
  it with `current_directive` rather than inventing a new method).
- Badge the window in the chat list ("Subconscious · steering") so the two
  meanings of "subconscious" in the product read as one system: the tab shows
  the tinyplace instance *output*, the Subconscious tab shows both instances'
  *health*.
- Cross-link back: the header links to the Subconscious tab for controls.

## 7.5 i18n

New keys (instance card titles/descriptions, run-review, steering header,
badge) go into `en.ts` **and real translations in all locale files**
(`ar bn de es fr hi id it ko pl pt ru zh-CN`). Gate: `pnpm i18n:check` +
`pnpm i18n:english:check`.

## 7.6 Tests

- Vitest, co-located:
  - `subconscious.ts` client: trigger passes `kind`; status parses
    `instances` and tolerates its absence (older core during rollout).
  - `useSubconscious`: per-kind triggering state; instances plumbed through.
  - `IntelligenceSubconsciousTab`: renders two cards from a stubbed status;
    tinyplace card disabled state when orchestration is off; per-card Run now
    dispatches the right kind.
  - `TinyPlaceOrchestrationTab`: steering header renders directive/empty
    states; run-review calls trigger with `tinyplace`.
- E2E (desktop spec, mock backend): Subconscious tab shows both cards;
  triggering the tinyplace review surfaces a directive message in the
  orchestration Subconscious window (mock `__admin/behavior` scripted).

## 7.7 Slicing

Two commits minimum: (a) types/client/hook + Subconscious tab cards,
(b) orchestration tab steering header + cross-links. i18n rides with each.
This phase can start as soon as phase 4's RPC shape is merged; it does not
depend on phases 5–6.
