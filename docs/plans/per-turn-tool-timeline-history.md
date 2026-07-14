# Per-turn tool-timeline history (draft)

**Status:** draft / design. Not yet implemented.
**Owners:** conversations UI + threads core.
**Related work (already shipped on this branch):**

- Coalescing repeated timeline rows + collapsing settled insights (`ToolTimelineBlock`).
- Streamed narration kept as an in-flow bubble (`Conversations.tsx`).
- `chat_interim` — mid-turn narration persisted as real interleaved chat messages (`progress_bridge.rs` + `ChatRuntimeProvider`).

## Problem

Each answer in a multi-turn thread should keep its own "Agentic task insights"
trail (the tools/subagents that produced it). Today the trail is lost for every
turn but the latest:

- The frontend holds **one** timeline array per thread
  (`toolTimelineByThread[threadId]`, `chatRuntimeSlice.ts:567`) and renders it
  once, anchored after the last user message (`Conversations.tsx` — the
  `lastUserMessageId ? agentInsights : null` anchor). Each new send wipes it
  (`setToolTimelineForThread([])`).
- The core persists **one** turn-state snapshot per thread, whole-file
  overwrite — "latest snapshot wins" (`turn_state/store.rs:1-6`,
  `snapshot_path` keyed by `hex(thread_id)` at `store.rs:210-216`). A
  `Completed` snapshot is kept only until the next turn on the same thread
  overwrites it (`turn_state/types.rs:26-30`, `mirror.rs:466-479`).

So on reload, scrolling up shows past answers with **no** process trail, and the
single current trail always sits at the bottom.

The `chat_interim` work already fixed the narration half of this (narration is
now a durable thread message). This design covers the remaining half: the
**tool timeline** per turn.

## Current data flow (anchors)

- Snapshot type: `TurnState { thread_id, request_id, lifecycle, streaming_text,
  thinking, tool_timeline: Vec<ToolTimelineEntry>, transcript: Vec<TranscriptItem>,
  task_board }` — `turn_state/types.rs:291-320`. Each turn already carries a
  unique `request_id` (`TurnState::started` / `TurnStateMirror::new`,
  `progress_bridge.rs:290-291`).
- Mirror: `TurnStateMirror::observe` folds `AgentProgress` into the snapshot;
  `TurnCompleted` marks `lifecycle = Completed` and keeps the snapshot
  (`mirror.rs:466-479`).
- Store: `put` whole-file overwrite (`store.rs:40-73`); `get`/`delete`/`list`/
  `clear_all`/`mark_all_interrupted` (`store.rs:76-194`); path keyed by thread
  (`store.rs:210-216`).
- RPC surface: `GetTurnStateRequest/Response`, `ListTurnStatesResponse`,
  `ClearTurnStateRequest/Response` (`turn_state/types.rs:322-358`,
  re-exported `turn_state/mod.rs:16-20`).
- Frontend consumer: `threadApi.getTurnState(threadId)` (`threadApi.ts:125`) →
  `hydrateRuntimeFromSnapshot` (`chatRuntimeSlice.ts:1482,1672-1681`), which
  writes the single `toolTimelineByThread[threadId]`.

## Proposed design

Keep a **bounded ring of completed snapshots per thread**, keyed by
`request_id`, plus the existing single "live/latest" snapshot. Anchor each
completed turn's timeline to the answer message(s) that turn produced.

### 1. Store: key by turn, keep the latest pointer

- Change `snapshot_path` to a per-turn file:
  `…/turn_states/<hex(thread_id)>/<request_id>.<ext>` (a directory per thread).
- `put` writes `<request_id>.json` (atomic tmp+rename, unchanged durability).
- Add `put_completed` / retention: on writing a `Completed` snapshot, prune the
  thread's directory to the newest `N` completed turns (propose `N = 20`) by
  `completed_at`, so history stays bounded (mirrors the timeline
  `REGISTRY_SOFT_CAP` philosophy — never unbounded).
- Keep a `latest` pointer for the in-flight/most-recent turn: either a
  `latest.json` symlink-free copy, or resolve "latest" by scanning the dir for
  the max `started_at`. A pointer file avoids a dir scan on the hot
  `get_latest` path.
- `get(thread_id)` → latest (back-compat for the current single-turn RPC).
- New `get(thread_id, request_id)` and `list_completed(thread_id)` (metadata
  only: `request_id`, `lifecycle`, `started_at`, `completed_at`, counts — not
  the full `tool_timeline`, to keep the list cheap).
- `mark_all_interrupted` (startup) and `clear`/`delete` operate over the
  per-thread directory. `mark_all_interrupted` still skips
  `Completed`/`Interrupted` (`store.rs:170-194`).

### 2. Turn ↔ message anchoring

The frontend must map each completed turn to the answer bubble(s) it produced.
Two options — pick **B**:

- **A. Anchor by user message.** Render a turn's timeline above the user message
  that triggered it. Requires recording the triggering user `message_id` on the
  snapshot. Simple but places the trail above the *question*, not the *answer*.
- **B. Anchor by produced assistant message (recommended).** Stamp the assistant
  messages a turn appends with its `request_id`. `addInferenceResponse`
  (`threadSlice.ts:185`) and the `chat_segment` / `chat_done` / `chat_interim`
  handlers all know `request_id`; thread it into `extraMetadata.requestId` and
  persist it (already an open field on `ThreadMessage.extraMetadata`). Then the
  frontend groups messages by `requestId` and renders that turn's timeline above
  the first assistant message of the group.

Option B is reload-coherent: messages already reload from the thread store with
their `extraMetadata`, and each turn's timeline reloads from its per-turn
snapshot — no divergence between live and reloaded views.

### 3. RPC + frontend

- New RPC `threads.turn_state_list` → `[{ requestId, lifecycle, startedAt,
  completedAt, toolCount, subagentCount }]` and `threads.turn_state_get`
  (`threadId`, `requestId`) → full `PersistedTurnState`. Keep the existing
  `get_turn_state(threadId)` for the live turn.
- Frontend store: replace `toolTimelineByThread: Record<threadId, Entry[]>` with
  `toolTimelineByThread: Record<threadId, Record<requestId, Entry[]>>` plus a
  `liveRequestIdByThread` pointer. `hydrateRuntimeFromSnapshot` writes under the
  turn's `requestId`; `setToolTimelineForThread` targets the live `requestId`
  and no longer wipes history on send.
- Render loop (`Conversations.tsx`): for each assistant message that is the first
  of its `requestId` group, render `<ToolTimelineBlock entries={byRequest[rid]}>`
  above it (collapsed-when-settled behavior already shipped). The live turn keeps
  its current in-flight rendering.
- On thread open, call `turn_state_list`, then lazily `turn_state_get` a turn's
  full timeline the first time its (collapsed) insights block is expanded — the
  list is cheap; full timelines load on demand so opening a long thread doesn't
  fetch dozens of full snapshots.

### 4. Migration / back-compat

- Old single-file snapshots (`<hex(thread_id)>.json`) are read once and migrated
  into `<hex(thread_id)>/<request_id>.json` on first access; if `request_id` is
  absent on a legacy snapshot, key it `legacy` and treat it as the thread's one
  historical turn.
- Messages without `extraMetadata.requestId` (pre-migration) fall back to the
  current single-anchor behavior (render the live/latest timeline once), so old
  threads degrade gracefully rather than losing their trail.

### 5. Reload coherence (the invariant to protect)

Live and reloaded views must render identically. Achieved because both sides key
on `requestId`: assistant messages carry it in `extraMetadata`; timelines are
stored/fetched per `requestId`. No in-session-only state — the failure mode this
codebase repeatedly warns about (e.g. the `preserveLiveSubagentProse` comments)
is avoided.

## Testing plan

- **Store (Rust):** per-turn put/get/list, retention prunes to `N`, latest
  pointer resolves correctly, legacy single-file migration, `mark_all_interrupted`
  over the per-thread dir. Extend `turn_state/store.rs` tests.
- **Mirror (Rust):** a `Completed` snapshot is retained under its `request_id`
  and a subsequent turn does not overwrite it.
- **Frontend:** `hydrateRuntimeFromSnapshot` writes under `requestId`; the render
  loop groups messages by `requestId` and renders one timeline per turn above the
  first assistant bubble; legacy messages (no `requestId`) fall back to the single
  anchor. Extend `Conversations.render.test.tsx` + `chatRuntimeSlice` tests.

## Rollout

1. Store keyed by `request_id` + retention + legacy migration (Rust, behind the
   existing single-turn RPC — no UI change yet).
2. Stamp assistant messages with `extraMetadata.requestId` (frontend, additive).
3. New `turn_state_list` / `turn_state_get(requestId)` RPCs.
4. Frontend store keyed by `requestId` + per-turn render, with legacy fallback.

Each step is independently shippable; the UI only changes at step 4.

## Estimated size

Medium-large. Step 1 is the core risk (persistence format + migration); steps
2–4 are mechanical but touch the render loop and the slice shape. Recommend
landing steps 1–3 first (invisible), then step 4 behind quick manual QA on a
multi-turn thread.
