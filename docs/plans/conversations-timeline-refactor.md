# Conversations Timeline Refactor — Audit & Plan

Status: **draft — approved for implementation, not yet started**
Scope: `app/src/pages/Conversations.tsx` and the conversation-timeline data layer (frontend + Rust turn-state persistence).
Companion: [`per-turn-tool-timeline-history.md`](per-turn-tool-timeline-history.md) (adopted wholesale as the backend track).

## Context

`app/src/pages/Conversations.tsx` (3,302 lines, one function component) is the app's reusable conversation panel — used by `Accounts.tsx` (page + sidebar variants) and `features/human/HumanPage.tsx` (sidebar + mic-cloud + projectThreadList). It is disorganized and buggy in four dimensions:

**How it renders tool calls / thoughts / subagents** — five parallel state families composed ad hoc in one render function: durable messages (`threadSlice`, server order), a **single tool-timeline array per thread** (`chatRuntimeSlice.toolTimelineByThread`), a streaming-preview tail (`streamingAssistantByThread`, last 120 chars), parallel subagent streams (`parallelStreamsByThread`), and an inference-status line — each with its own conditional render gates (`hideAgentInsights` × `isSending` × anchoring produces 4+ fallback paths, L1775–1842).

**How it orders** — no timestamps or sequence ids. Timeline entries carry only `round` + array insertion order. The single timeline is positionally **anchored after the last user message** (`Conversations.tsx:1756, 2533`, fallback `2669` for proactive threads). Each send **wipes** the timeline, so past turns permanently lose their process trail.

**How it stores & streams** — `ChatRuntimeProvider.tsx` (1,664 lines) merges ~28 snake_case socket events (`tool_call`, `subagent_*`, `chat_interim`, `text_delta`, …) via `store.getState()` + find-row + **full-array-replace** in ~10 handlers. **Two parallel dedup mechanisms** (provider-local `seenChatEventsRef` string-key TTL map + reducer-level id dedup in `hydrateRuntimeFromRunLedger`) with **divergent row-id schemes** (live: `${thread}:subagent:${task}:${tool}` vs ledger: `subagent:${runId}`), plus provider-local `segmentDeliveriesRef` and the `preserveLiveSubagentProse` graft — a documented live-only-state workaround.

**Tab-switch context loss** — the Rust store persists **one** turn snapshot per thread (whole-file overwrite, `src/openhuman/threads/turn_state/store.rs`), so thread/tab switch rehydrates only the latest turn; live subagent prose, past-turn trails, and the streaming tail are lost.

**Reference patterns adopted** (from codex `codex-rs/protocol` + TUI research; hermes-agent confirmed as the minimal counter-example; vendored `tinychannels` is transport-only — its `ChannelOutputEvent` vocabulary and `RunLedger`/`ConversationStore` host traits stay compatible, the rich timeline model remains OpenHuman-owned):

1. **Two-layer model**: ephemeral streaming events (deltas, begin/end pairs keyed by `call_id`) vs **durable timeline items with stable ids**; deltas mutate the item with the matching id; persist only durable items; rehydrate by chronological replay.
2. **Typed item taxonomy**: one discriminated union, one React component per kind.
3. **Ordering**: stable per-item id + monotonic `seq`; per-turn grouping via `requestId`; subagents as flat items with `parentId`, nesting rendered not stored.

---

## Target architecture

### 1. Unified `TimelineItem` model — a selector projection, not a new slice

`threadSlice` and `chatRuntimeSlice` remain sources of truth; a memoized selector (`selectTimelineForThread`) composes them into one ordered `TimelineItem[]`. This avoids a big-bang slice migration and lets each phase land independently.

```ts
// app/src/features/conversations/timeline/types.ts
interface TimelineItemBase {
  id: string;      // stable: message id, or unified runtime row id (§3)
  turnId: string;  // requestId; 'legacy' for pre-migration turns
  seq: number;     // ordering within turn (reducer-assigned now, backend-stamped in Phase 4)
  threadId: string;
}
type TimelineItem = TimelineItemBase & (
  | { kind: 'userMessage';      message: ThreadMessage }
  | { kind: 'assistantMessage'; message: ThreadMessage; interim: boolean }  // chat_interim narration
  | { kind: 'streamingText';    text: string; streamId?: string }           // ephemeral, live turn only
  | { kind: 'reasoning';        text: string; settled: boolean }
  | { kind: 'toolCall';         callId: string; name: string; status: 'running'|'ok'|'error'; args?: unknown; result?: unknown; round?: number }
  | { kind: 'subagentActivity'; taskId: string; parentCallId?: string; children: string[] } // flat store, rendered nested
  | { kind: 'approvalRequest' | 'plan' | 'workflowProposal'; /* wraps existing card payloads */ }
);
```

**Ordering**: turns order by first-message position in the thread; items within a turn by `seq`. Anchoring becomes structural — a turn group renders `[userMessage, ...processItems, ...assistantMessages]` — replacing the last-user-message positional hack. Messages without `requestId` fall into a single `legacy` turn that reproduces today's single-anchor behavior. A turn with no `userMessage` (proactive threads) is first-class: process items render before its first assistant message.

### 2. File layout — `app/src/features/conversations/` (matches `features/human/`; all files ≤ ~500 lines)

```
app/src/features/conversations/
  ConversationsPage.tsx          # page-shell variant
  ConversationsSidebar.tsx       # sidebar variant (Accounts, HumanPage)
  index.ts                       # public exports = reusability contract
  timeline/
    types.ts                     # TimelineItem union
    selectors.ts                 # projection + turn grouping (reselect-memoized)
    ConversationTimeline.tsx     # pure renderer: TimelineItem[] → one component per kind
    items/                       # UserMessageItem, AssistantMessageItem, ToolCallItem,
                                 #   ReasoningItem, SubagentActivityItem, StreamingTailItem
    TurnInsightsGroup.tsx        # collapsed-header/expanded-body per-turn wrapper (lazy-load)
  composer/
    Composer.tsx / MicCloudComposer.tsx / useComposerState.ts / composerSendDecision.ts
  threadList/
    ThreadList.tsx / threadFilter.ts
  components/                    # existing subcomponents relocated as-is
    (ToolTimelineBlock, SubagentDrawer, TaskKanbanBoard, AgentProcessSourcePanel, …)
app/src/pages/Conversations.tsx  # thin re-export shim during migration; deleted in Phase 6
```

**Reusability contract** (`index.ts`): `<ConversationsPage>`, `<ConversationsSidebar composer projectThreadList>`, and `<ConversationTimeline items onAction>` for future embedders. The `AgentChatPanel` export (Conversations.tsx:3302) is renamed to resolve the collision with the unrelated `components/settings/panels/AgentChatPanel.tsx`; `Accounts.tsx:25` updated.

### 3. Streaming/merge consolidation

- All merge logic moves into `chatRuntimeSlice` **reducers** — one typed action per event family (`toolEventReceived`, `subagentEventReceived`, `textDeltaReceived`, `turnLifecycleReceived`). `ChatRuntimeProvider` shrinks to parse-and-dispatch (~300 lines); no `getState()`, no full-array rebuilds.
- **One dedup mechanism**, reducer-level, on a unified row-id scheme: `${threadId}:${requestId}:${kind}:${callId|taskId|streamId}` — used identically by live handlers, `hydrateRuntimeFromSnapshot`, and `hydrateRuntimeFromRunLedger`. Kills the live-vs-ledger id divergence; `preserveLiveSubagentProse` becomes an ordinary upsert (merge rule: persisted fields win for settled rows, live fields win for streaming fields).
- **Backend `seq` envelope** (Phase 4): `progress_bridge.rs` stamps a per-request monotonic `seq: u64` + `request_id` on every event; reducers dedup by `(requestId, seq)`. Reconnect-safe: replayed events with `seq <= lastSeq` are dropped; missed events recovered by re-hydration on reconnect (kept). Until then, the TTL-map dedup moves verbatim into the reducer as the interim mechanism.
- `segmentDeliveriesRef` reconstruction becomes per-request segment state in the slice, cleared on `chat_done`.

### 4. Per-turn persistence (backend)

Adopt `per-turn-tool-timeline-history.md` as written: per-turn files `turn_states/<hex(thread_id)>/<request_id>.json` with retention (N=20) + `latest` pointer, `turn_state_list` / `turn_state_get(requestId)` RPCs, idempotent legacy single-file migration, `extraMetadata.requestId` stamped on assistant messages (Option B anchoring). **Amendment**: persist `seq` on `PersistedToolTimelineEntry` (Rust `turn_state/types.rs` + `app/src/types/turnState.ts`) so replayed snapshots order identically to live streams.

**Sequencing: frontend-first.** The projection, component split, and reducer consolidation all work against the current single-snapshot backend (everything lands in the `legacy`/live turn). The ring store then slots in underneath without touching the renderer.

---

## Phases (each a small PR, tests green throughout; branch off `upstream/main`, small focused commits)

### Phase 0 — Land this plan
This document, committed to `docs/plans/`.

### Phase 1 — Mechanical extraction, no behavior change
Create the `features/conversations/` skeleton; relocate `pages/conversations/{components,hooks,utils}`; extract ThreadList, Composer/MicCloudComposer/useComposerState, and the page/sidebar shells out of `Conversations.tsx`. `pages/Conversations.tsx` becomes a shim with the old props so `Accounts.tsx`/`HumanPage.tsx` are untouched. Rename `AgentChatPanel`. Shared thread/selection state via a `ConversationsContext` to avoid prop-drilling explosions.
**Guardrails**: `Conversations.render.test.tsx` + 4 sibling tests, Accounts/HumanPage tests pass unmodified (import paths only).

### Phase 2 — `TimelineItem` projection + `<ConversationTimeline>`
Add `timeline/types.ts`, `timeline/selectors.ts` (`legacy`-turn fallback ⇒ behaviorally identical to today's anchor logic), `ConversationTimeline.tsx` + `items/*` (wrapping existing `ToolTimelineBlock`, `AgentMessageBubble`, cards). Replace the render loop (~L2400–2700 of the old file). `hideAgentInsights` filters kinds in the selector, not components.
**New tests**: `selectors.test.ts` (ordering, legacy fallback, empty thread, **proactive thread with no user message** — must reproduce the L2669 fallback), `ConversationTimeline.test.tsx` (snapshot per kind).

### Phase 3 — Reducer-side merge/dedup consolidation
Typed actions in `chatRuntimeSlice`; move each provider handler's merge body into reducers; move `seenChatEventsRef` + `segmentDeliveriesRef` into slice state; unify row ids everywhere; replace `preserveLiveSubagentProse` with the field-level upsert. Verbose debug logging on every dedup drop / merge decision (repo convention).
**Guardrails**: `ChatRuntimeProvider.test.tsx` (assertions rewritten to dispatched actions), `chatRuntimeSlice.test.ts`, `chatRuntimeSlice.toolFailure.test.ts`, `SubagentDrawer.test.tsx`. **New tests**: duplicate-event replay is a no-op; live subagent prose survives snapshot hydration; reconnect-replay simulation.

### Phase 4 — Backend per-turn ring store (companion-plan steps 1–3)
Rust first (per AGENTS.md workflow Rust → RPC → UI): per-turn store layout + retention + legacy migration in `turn_state/store.rs`, mirror retention in `mirror.rs`, `seq` stamping in `progress_bridge.rs`, `seq` on `turn_state/types.rs`; `turn_state_list` / `turn_state_get(requestId)` RPCs; `extraMetadata.requestId` stamping (threadSlice `addInferenceResponse` + `chat_segment`/`chat_done`/`chat_interim` paths); wire types in `app/src/types/turnState.ts` + `threadApi.ts`. Invisible to UI.
**Tests**: extend `store_tests.rs` (put/get/list, prune to N, latest pointer, idempotent legacy migration, `mark_all_interrupted` over dir), `mirror_tests.rs`; JSON-RPC E2E for the new methods.

### Phase 5 — Per-turn frontend (companion-plan step 4)
`toolTimelineByThread` → `Record<threadId, Record<requestId, rows>>` + `liveRequestIdByThread`; send no longer wipes history; dedup by `(requestId, seq)`, drop the interim TTL logic. Selector groups by real `requestId`; `TurnInsightsGroup` lazy-fetches `turn_state_get(requestId)` on first expand; thread-switch hydration = `turn_state_list` + latest snapshot only.
**New tests**: multi-turn render (each turn keeps its trail), lazy-load, legacy-message fallback.

### Phase 6 — Cleanup
Delete the `pages/Conversations.tsx` shim (point Accounts/HumanPage at `features/conversations`); delete dead state (`parallelStreamsByThread` if subsumed by `streamingText` items, `preserveLiveSubagentProse`, provider refs). i18n keys unchanged (all strings already via `useT`).

---

## Tab-switch context loss — cause → fix

| Lost today | Cause | Fixed by |
|---|---|---|
| Past turns' process trails | single array wiped on send + single-snapshot store | Phases 4–5 |
| Live subagent prose after rehydrate | hydration clobbers live-only rows (`preserveLiveSubagentProse` workaround) | Phase 3 (unified ids + field-level upsert) |
| Streaming tail / interim segments | provider-local refs + latest-snapshot-only hydration | Phase 3 (segment state in slice) + Phase 4 |
| Duplicate/ghost rows after reconnect | TTL dedup misses + id-scheme divergence | Phase 3 interim, Phase 4 `seq` definitive |

## Verification

- Per phase: `pnpm test` (guardrail tests listed above), `pnpm typecheck`, `pnpm lint`; Phase 4 adds `cargo test` / `pnpm test:rust` + `tests/json_rpc_e2e.rs`; ≥80% diff coverage (CI Lite gate).
- Manual QA after Phases 3 and 5 (`pnpm dev:app`): multi-turn thread with tools + subagents; switch tab mid-stream and back; kill/restore socket mid-turn; proactive thread with no user message; `hideAgentInsights` sidebar variant in HumanPage; app restart on a 5-turn thread — all trails restored, ordered identically to live.

## Risks

- **Reconnect replay before `seq` exists** — keep the TTL-equivalent dedup in the reducer until Phase 4 lands; do not delete early.
- **Proactive threads (no user message)** — first-class in the projection; tested in Phase 2.
- **Snapshot migration** — legacy single-file read must be idempotent; keep the old read path one release.
- **Selector performance on long threads** — reselect memoization per thread; collapsed turns hold metadata only (lazy-load bodies).
- **`round` semantics** — `ToolTimelineBlock` coalescing keys off `round`; keep `round` alongside `seq`, don't replace it.

## Critical files

- `app/src/pages/Conversations.tsx` (3,302 ln — split & shim)
- `app/src/providers/ChatRuntimeProvider.tsx` (1,664 ln — shrink to dispatch)
- `app/src/store/chatRuntimeSlice.ts` (1,698 ln — reducer-side merges, per-turn shape)
- `app/src/store/threadSlice.ts` (`addInferenceResponse` requestId stamping)
- `app/src/types/turnState.ts`, `app/src/services/api/threadApi.ts` (wire types/RPCs)
- `src/openhuman/threads/turn_state/{store.rs,mirror.rs,types.rs}` (ring store, seq)
- `src/openhuman/channels/providers/web/progress_bridge.rs` (seq envelope)
- `docs/plans/per-turn-tool-timeline-history.md` (adopted backend design)
