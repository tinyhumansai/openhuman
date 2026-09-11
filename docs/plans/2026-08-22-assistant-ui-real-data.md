# Wiring the assistant-ui chat surface to real OpenHuman data

**Status:** implementation complete on `fix/ui-shdcdn`; branch-wide verification
is pending unrelated stale AI-settings tests.
**Prerequisite:** the mock surface landed on `fix/ui-shdcdn` (see "Starting point").

Current checkpoint:

- The product chat uses `AssistantUiRuntimeProvider`; the scripted runtime is now
  isolated to the dev-only assistant-ui demo.
- Real reasoning, tool timelines and subagent activity are projected into
  assistant-ui parts, including the running-args/final-result invariant.
- Context usage reads the selected thread's Redux token bucket, and cached input
  remains separate from fresh input. A provider catalog selection now carries
  its real context window into both the meter and the model override used by
  `chatSend`; unknown windows render as unknown.
- The composer reuses the persisted thread-goal controller, and `/` commands are
  projected from the shared command registry. `/clear` executes `chat.new`.
- Focused tests, typecheck, lint and both i18n gates pass. The migrated
  `Conversations.render.test.tsx` suite passes 59/59 and the combined
  assistant-ui/conversation/workflow set passes 158/158. Full `pnpm test`
  reaches unrelated stale `AIPanel.test.tsx` cases that still assert the former
  monolithic/radio settings UI instead of the current Providers/Routing tabs.
- `renderAssistantUiOnly` is removed. The voice-only `mic-cloud` embed still
  uses the legacy pane while the remaining non-plan affordances are ported;
  follow-up: [#5685](https://github.com/tinyhumansai/openhuman/issues/5685).

## What this is

At the start of this work, `Conversations` rendered the assistant-ui `Thread`
behind a temporary `renderAssistantUiOnly` switch and fed it from a scripted
offline adapter. That was deliberate: it let the _shape_ of the surface be built
and reviewed — streamed reasoning, tool calls, dispatched subagents, slash
commands, a context meter, and a thread goal — before any of it was load-bearing.

The normal text-chat path now renders that same `Thread` from OpenHuman's real
Redux/core seams. The `mic-cloud` embed retains its voice-specific legacy footer
until the controls tracked in #5685 have an assistant-ui equivalent.

This document is the other half: replacing each mock with the real seam. It is
written for an agent picking the work up cold, so every seam below names the file
and the RPC method rather than describing them.

**The through-line: none of these are new features.** OpenHuman already has a
goal domain, a cost domain, a token-usage RPC, a command palette and a
Redux-backed transcript projection. Four of the five tasks are _deletions_ —
removing a mock and pointing the component at machinery that already exists. Do
not build a second implementation of any of them.

## Starting point

| Piece             | File                                                                       | Backed by                    |
| ----------------- | -------------------------------------------------------------------------- | ---------------------------- |
| Scripted turn     | `app/src/lib/assistantUiMock/mockScript.ts`                                | fiction                      |
| Streaming adapter | `app/src/lib/assistantUiMock/mockChatModel.ts`                             | fiction                      |
| Subagent renderer | `app/src/lib/assistantUiMock/SubagentCall.tsx`                             | reads the mock's payload     |
| Chat surface      | `app/src/features/conversations/components/AssistantUiChat.tsx`            | `useLocalRuntime` + the mock |
| Context meter     | `app/src/features/conversations/components/composer/ContextWindowPill.tsx` | `MOCK_USAGE` constant        |
| Goal control      | `app/src/features/conversations/components/composer/GoalSelector.tsx`      | React `useState`             |
| Slash commands    | `slashCommands` prop on `Thread`                                           | two local closures           |

Two seams were added to the vendored `Thread`
([`app/src/components/assistant-ui/thread.tsx`](../../app/src/components/assistant-ui/thread.tsx))
so the app-specific parts stay out of it, and both survive this migration
unchanged:

- `ThreadComponents.ComposerExtras` — extra controls in the composer action row.
- `ThreadProps.slashCommands` — `/` commands, supplied by the host because a
  command's `execute` is host behaviour.

Keep using them. Putting OpenHuman logic back inside `thread.tsx` is what makes
the file impossible to re-pull from the registry.

---

## Task 1 — Runtime: `useLocalRuntime` → the existing external store

**Delete** the mock runtime; **do not** write a new bridge. One already exists
and is tested:

- [`app/src/providers/AssistantUiRuntimeProvider.tsx`](../../app/src/providers/AssistantUiRuntimeProvider.tsx)
  — mounts `useExternalStoreRuntime`, scoped to one thread by prop.
- [`app/src/providers/useOpenHumanExternalStore.ts`](../../app/src/providers/useOpenHumanExternalStore.ts)
  — the adapter.
- [`app/src/providers/assistantUiMessages.ts`](../../app/src/providers/assistantUiMessages.ts)
  — the Redux → assistant-ui projection. Read its header before touching it: the
  projection is **read-only**, and `chatRuntimeSlice` / `threadSlice` stay the
  single source of truth for messages, streaming, tool state and persistence.

### Steps

1. In `AssistantUiChat`, drop `useLocalRuntime`, `mockChatModelAdapter`,
   `buildSeedMessages` and `initialMessages`. Render `Thread` inside the
   existing `AssistantUiRuntimeProvider` instead.
2. Decide the thread scope explicitly. `AssistantUiRuntimeProvider` takes
   `threadId?: string | null` where **omitted** means "follow
   `state.thread.selectedThreadId`" and an explicit `null` means "this surface
   owns a thread it has not created yet". The main chat pane wants the former.
   Getting this wrong is a real bug with a real precedent — see the provider's
   header comment about the Workflow Copilot painting the home chat's messages.
3. Sending: the composer must reach the same path the legacy composer used.
   `useOpenHumanExternalStore.onNew` is the seam; verify it is wired before
   removing the legacy pane, not after. The adapter deliberately exposes no
   `onEdit`: `openhuman.threads_message_update` updates metadata only, not
   message content, so advertising assistant-ui's edit capability would create
   a button the core cannot honour.

### Watch for

- **Re-render cost.** `assistantUiMessages` has a performance property pinned as
  a test: converting the transcript while a token streams must not re-convert
  every message. `ChatThreadView.renderPerf.test.tsx` and
  `ChatThreadView.memoIdentity.test.tsx` exist for this. Run them.
- Once this lands, `/clear` (Task 4) can no longer call `runtime.thread.reset()`
  — the runtime is a projection and does not own the messages.

---

## Task 2 — Transcript parts: real reasoning, tools and subagents

The mock emits `reasoning`, `tool-call` and `text` parts directly. Real ones
arrive as core events through
[`app/src/services/chatService.ts`](../../app/src/services/chatService.ts) and
land in `chatRuntimeSlice`; `assistantUiMessages` projects them.

Relevant wire types already defined there: `ChatSegmentEvent`,
`ChatToolCallEvent`, `ChatToolResultEvent`, `ChatSubagentSpawnedEvent`,
`ChatSubagentDoneEvent`, `SubagentProgressDetail`, `TurnUsageWire`,
`SubagentUsageWire`, `ChatDoneEvent`.

### Steps

1. Confirm `assistantUiMessages` projects reasoning segments as
   `{ type: 'reasoning' }` parts. If it flattens them into text, the
   chain-of-thought group will never form and the reasoning UI stays dark.
2. Map subagent events onto a tool-call part named `task`, so
   `MockToolFallback` keeps routing them to `SubagentCall`. Rename
   `MockToolFallback` → `ChatToolFallback` and move it out of `lib/assistantUiMock/`
   when the mock directory goes.
3. Rework `SubagentCall` to read `SubagentProgressDetail` instead of the mock's
   payload. **Preserve the running/complete split exactly as it is**, and read
   the note in `mockChatModel.ts` before changing it: assistant-ui derives a tool
   call's status from _whether a result is present_, so live progress must ride
   on the streaming `args` and `result` must be set once, at the end. Writing
   progress into `result` marks the call complete the instant it starts, the tool
   group collapses, and a running delegation renders as finished. This was found
   the hard way; do not rediscover it.
4. Keep `MockToolGroup` (rename to `ChatToolGroup`): a group holding in-flight
   work opens itself, which is the only reason a dispatched delegation is visible
   while the answer streams.

### Watch for

- **Existing subagent UI.** `SubagentActivityBlock.tsx`, `SubagentToolCallRow.tsx`
  and `SubagentDrawer.tsx` already render this data in the legacy pane. Decide
  deliberately whether `SubagentCall` replaces them or wraps them; do not let two
  renderers of the same events drift.
- Real turns interleave differently from the script. The script dispatches both
  delegations early on purpose; real ordering is whatever the core emits.

---

## Task 3 — Context meter: `MOCK_USAGE` → real token accounting

`ContextWindowPill` takes a `ContextUsage` prop and renders it. The component
does not change; only its source does.

Two candidate sources, both real:

| Source                | File                                                                                                   | Shape                                                                |
| --------------------- | ------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------- |
| Live, per-session     | `chatRuntimeSlice` — `emptySessionTokenUsage`, `SubAgentUsage`                                         | already consumed by `app/src/components/chat/ComposerTokenStats.tsx` |
| Persisted, per-thread | `openhuman.threads_token_usage` via `app/src/services/api/threadUsageApi.ts` (`fetchThreadTokenUsage`) | `ThreadTokenUsage`                                                   |

### Steps

1. **Read `ComposerTokenStats.tsx` first.** It is the legacy composer's version
   of this exact control — per-agent breakdown, tokens · cost · runs — and it
   already solves the aggregation. Prefer reusing its selector over writing a
   second one.
2. Map onto `ContextUsage`: `used`, `limit`, `input`, `cachedInput`, `output`,
   `costUsd`. **`cachedInput` must stay separate from `input`** — a cache read is
   billed at a fraction of a fresh read, so cost cannot be derived from one token
   count. If the wire type does not carry cached input, extend it rather than
   folding the two together.
3. `limit` is the selected model's context window, not a constant. It has to
   follow the model picker; `context_window` on `TurnUsageWire` is `0` in some
   cases (see the comment there) — decide what the meter shows then, rather than
   dividing by zero.
4. Cost for a _thread_ may want `openhuman.cost_get_summary` from
   `src/openhuman/platform/cost/` instead of a per-turn sum. Check `README.md` in
   that directory before choosing.

### Watch for

- The pill's thresholds (amber > 75%, red > 90%) assume `limit` is real. With a
  placeholder limit they are theatre.

---

## Task 4 — Goal: local state → the thread-goal domain

**This one is almost entirely a deletion.** OpenHuman already has a per-thread
goal, end to end:

- UI: [`app/src/features/conversations/components/ThreadGoalChip.tsx`](../../app/src/features/conversations/components/ThreadGoalChip.tsx)
  — "Set goal" when empty, status + objective when set, click to edit.
- Client: [`app/src/services/api/threadGoalApi.ts`](../../app/src/services/api/threadGoalApi.ts)
  — `openhuman.thread_goals_get` / `_set` / `_complete`.
- Core: `src/openhuman/threads/goals/` — set, get, complete, pause, resume,
  clear, with `objective`, `token_budget`, `result`. Persisted per thread under
  `<workspace>/thread_goals/<hex(thread_id)>.json`.

### Steps

1. **Delete `GoalSelector.tsx`.** Render `ThreadGoalChip` from `ComposerExtras`
   instead. It already has the dialog, the RPC calls and the empty state.
2. `ThreadGoalChip` owns its own open state, while `/goal` needs to open it from
   outside. Lift open state to a controlled prop, mirroring how `GoalSelector`
   did it — that was the whole reason `open`/`onOpenChange` were props there.
3. Delete the `conversations.composer.goal.*` i18n keys from **all 14 locale
   files** once `GoalSelector` is gone; `ThreadGoalChip` has its own.

### Watch for

- A thread goal is not the global long-term goals list on the Intelligence tab.
  `ThreadGoalChip`'s header says so explicitly. Do not merge them.

---

## Task 5 — Slash commands: local closures → the command registry

`/clear` and `/goal` are currently closures in `AssistantUiChat`, and `/clear`
calls `runtime.thread.reset()` — which stops working after Task 1.

OpenHuman already has a command surface:
[`app/src/components/commands/CommandProvider.tsx`](../../app/src/components/commands/CommandProvider.tsx),
`CommandPalette.tsx`, `CommandScope.tsx`.

### Steps

1. Source the composer's `/` list from the command registry so a command is
   defined once and reachable from both the palette and the composer.
2. Reimplement `/clear` against the real transcript. It must go through whatever
   `threadSlice` / core RPC owns clearing a thread — **not** a runtime reset,
   which after Task 1 would desynchronise the projection from Redux.
3. Decide what `/clear` means before implementing: clear the _view_, archive the
   thread, or start a new one. The mock's behaviour (empty the transcript in
   place) is the least likely to be right for real data, because the messages are
   persisted.
4. Any command added here needs i18n keys in all 14 locales (see below).

---

## Cross-cutting

### i18n

`pnpm i18n:check` currently passes with `missing: 0` for **every** locale, so a
key added to `en.ts` alone will fail it. Every new key needs all 14 files.
Existing generic keys (`common.save`, `common.cancel`, `common.remove`) already
cover the obvious buttons. Translation values must not contain em dashes
(`U+2014`).

Gates: `pnpm i18n:check`, `pnpm i18n:english:check`, plus the coverage test.

### Styling

Do not reintroduce per-component margins in the transcript. The assistant message
owns one vertical rhythm (`[&>*+*]:mt-3`, applied at both the message and
chain-of-thought levels, with `mb-0` neutralising `reasoning-root`'s `mb-4`).
Measured before that change, gaps ran 16 / 0 / 0 / 16 / 0 / 0 px; after, a flat
12 px. A new block that brings its own margin breaks the rhythm silently.

Three global CSS facts this surface depends on, all in `app/src/index.css`:

- Default border colour is `var(--color-border)`, not Tailwind v4's
  `currentColor` and not the `gray-200` the v4 upgrade pinned. 18 vendored
  components use a bare `border`; with a fixed grey they drew a bright hairline
  in dark mode.
- `html textarea { border-width: 0 }` — `@tailwindcss/forms` resets bare
  textareas to a square grey border the composers never asked for.
- `html textarea:focus` zeroes the plugin's ring for the same reason. Neither
  rule uses `!important`, so a component setting its own `border` or
  `focus:ring-*` still wins. Do not "simplify" these into `!important`.

The shadcn semantic tokens live in `app/src/styles/shadcn-tokens.css` as aliases
over the canonical palette. `shadcn add` was run but never `shadcn init`, so that
file and the border-colour default are two halves of the same missing setup.

### Tests

The mock surface shipped **without** test coverage — that is a known gap, not a
precedent. Each task above needs at least:

- Task 1: the existing `ChatThreadView.*.test.tsx` suite green, including the
  render-perf and memo-identity tests.
- Task 2: a test that a delegation with no `result` renders as running and its
  group is open — this is the invariant that broke once already.
- Task 3: a test that cached input is not folded into input.
- Task 4/5: coverage of the controlled-open path and of `/clear`'s chosen
  semantics.

Run `pnpm test`, `pnpm typecheck`, `pnpm lint` and the i18n gates before opening
a PR. Diff coverage on changed lines must be ≥ 80%.

## Definition of done

- `app/src/lib/assistantUiMock/` is deleted.
- `AssistantUiChat` mounts no `useLocalRuntime` and imports nothing from a mock.
- The context meter, the goal control and every slash command read live data.
- `renderAssistantUiOnly` can be removed along with the legacy pane, or a
  follow-up issue explains what still blocks it. The constant is gone; [#5685](https://github.com/tinyhumansai/openhuman/issues/5685)
  tracks the remaining voice-only fallback and legacy affordances.
