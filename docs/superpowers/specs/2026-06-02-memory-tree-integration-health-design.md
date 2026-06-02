# Memory Tree — per-integration health strip (issue #2763)

**Status:** approved, ready for implementation plan
**Issue:** [tinyhumansai/openhuman#2763](https://github.com/tinyhumansai/openhuman/issues/2763)
**Parent umbrella:** #1856 Part 3
**Depends on (already merged):** #2719 (status panel), #1250 (per-source sync status)

## Goal

Give operators a quick at-a-glance per-integration health view, directly under the four-tile Memory Tree status panel, so a stalled / low-volume tree can be traced to a single integration (e.g. Gmail) rather than the pipeline as a whole.

## Non-goals

- Replacing or shrinking `MemorySourcesRegistry` (the full sources list stays below as-is — it owns add / sync / remove flows).
- Per-provider `Error` attribution. This needs new core work (job → source linkage) and is **deferred** to a follow-up issue tracked in the PR body.
- Team-scoped wiki silos (Part 2 of #1856 — blocked on FR9, not in scope here).

## Architecture

Frontend-only diff. Reuses the existing `openhuman.memory_sync_status_list` RPC (shipped by #1250) as the data source. The 575-line `MemorySourcesRegistry` remains as the canonical configurable-sources view; this strip is a smaller, health-focused readout colocated with the pipeline-status tiles.

```text
┌─────────────────────────────────────────────────────────────────┐
│ MemoryTreeStatusPanel                                           │
│ ┌─Status─┬─LastSync─┬─Chunks─┬─Wiki──┐                          │
│ │   …    │    …     │   …    │   …   │                          │
│ └────────┴──────────┴────────┴───────┘                          │
│                                                                 │
│ ┌── Per-integration health ──────────────────────────────────┐  │
│ │ [icon] slack            5,231 chunks · 3 min ago  ● Active │  │
│ │ [icon] gmail              842 chunks · 2 hr ago   ● Stale  │  │
│ │ [icon] notion              45 chunks · 5 d ago    ● Stale  │  │
│ └────────────────────────────────────────────────────────────┘  │
│                                                                 │
│ ┌─ Auto-sync toggle ──────────────────────────────[switch]──┐   │
│ └───────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────────┐
│ MemorySourcesRegistry (existing, unchanged)                     │
└─────────────────────────────────────────────────────────────────┘
```

## Data flow

1. `useMemoryTreeStatus` (existing hook in `MemoryTreeStatusPanel.tsx`) extends `fetchOnce` to call both:
   - `memoryTreePipelineStatus()` (existing)
   - `memorySyncStatusList()` (existing — wraps `openhuman.memory_sync_status_list`)
   in parallel via `Promise.all`. Returned object gains an `integrations: MemorySyncStatus[]` field.
2. Adaptive polling (1.5s while syncing, 4s otherwise) — the existing cadence — drives both fetches. One timer, one re-render.
3. A new internal sub-component `<IntegrationHealthStrip>` (same file, not exported) consumes `integrations` and renders the list. Lives inside `MemoryTreeStatusPanel` between the tile grid and the auto-sync toggle row.

## Status mapping (pure TS, no core change)

```ts
type IntegrationHealth = 'active' | 'stale';

function classifyIntegration(s: MemorySyncStatus): IntegrationHealth {
  return s.freshness === 'active' ? 'active' : 'stale';
}
```

- `freshness === 'active'` → **Active** (chunk within last 30 s)
- `freshness === 'recent' | 'idle'` → **Stale**
- **Error** state intentionally omitted; see "Deferred" below.

## UI details

- One row per `MemorySyncStatus`. Icon from a small built-in `PROVIDER_ICONS` map inside `MemoryTreeStatusPanel.tsx` (keyed by sync-provider name — `slack` / `gmail` / `notion` / …, distinct from `SOURCE_KIND_ICONS` which keys by `SourceKind`); fallback to a generic `🔌` glyph for unknown providers.
- Provider name: friendly label via `SOURCE_KIND_LABEL_KEYS[provider]` when present, else the raw `provider` string.
- "5,231 chunks · 3 min ago" — chunk count + relative time, reusing the existing `formatRelativeMs()` helper from `MemoryTreeStatusPanel.tsx`.
- Status pill: dot color reuses `statusDotClass` semantics — sage-400 for `active`, stone-400 for `stale`.
- Empty state: `data-testid="memory-tree-integrations-empty"`, single line "No integrations connected" matching `MemorySources.tsx` convention.
- Scroll: `max-h-48 overflow-y-auto` once past ~5 rows so the strip never dominates the panel.

## i18n

New keys colocated with `memoryTree.status.*` in `app/src/lib/i18n/en.ts`:

| key | English |
| --- | --- |
| `memoryTree.status.integrationsTitle` | Per-integration health |
| `memoryTree.status.integrationsEmpty` | No integrations connected |
| `memoryTree.status.integrationActive` | Active |
| `memoryTree.status.integrationStale` | Stale |
| `memoryTree.status.integrationChunks` | {count} chunks |

All 13 non-English locales (`ar`, `bn`, `de`, `es`, `fr`, `hi`, `id`, `it`, `ko`, `pl`, `pt`, `ru`, `zh-CN`) get **real translations** in the same PR, per CLAUDE.md i18n rule. `pnpm i18n:check` and `pnpm i18n:english:check` must pass.

## Testing

**Vitest** (`MemoryTreeStatusPanel.test.tsx`):

1. Renders integration rows from `memory_sync_status_list` (mocked).
2. Renders empty state when list is empty.
3. Status mapping: a row with `freshness='active'` shows `Active`; `freshness='recent'` and `freshness='idle'` both show `Stale`.
4. Icon fallback for unknown provider doesn't throw.
5. Relative-time label uses `formatRelativeMs` (frozen clock).
6. Shared poll: both `memoryTreePipelineStatus` and `memorySyncStatusList` are called on the same tick (mock both, advance fake timers, assert call counts).

**Coverage:** changed-lines ≥ 80 % (CI gate). The mapping + render branches are small and trivially testable; achievable.

**No new Rust tests** — no Rust changed. Existing `memory_sync_status_list` test coverage continues to validate the wire shape.

**No new E2E spec** — covered by the existing intelligence smoke test plus the unit tests above. (E2E for a render-only sub-component is overkill.)

## Deviations from issue acceptance criteria

Will be noted explicitly in PR body so reviewers see them up front:

1. **AC #1** says `memory_tree_pipeline_status` returns an `integrations` array. We **don't** extend that RPC; we consume `memory_sync_status_list` instead. Same data, cleaner contract, no schema bump.
2. **AC #2** says status is `Active / Stale / Error`. We ship **Active / Stale** only. Per-provider `Error` requires new core work (`mem_tree_jobs` has no `source_kind` / `source_id` column today; we'd have to parse `payload_json` per row or add a column). Deferred to a follow-up issue filed alongside this PR.

Remaining ACs (list renders below status panel, empty state, polling shares parent, i18n parity, ≥80 % coverage) are met as specified.

## Risks

- **Visual crowding** if many providers are connected. Mitigated by `max-h-48 overflow-y-auto`.
- **Empty `memory_sync_status_list` when chunks haven't flowed yet** — the strip will render empty even for a freshly-installed integration. Acceptable for v1 (the issue's same gap); when per-provider error tracking lands, "configured but never produced chunks" can be its own state.

## Out of scope (filed as follow-up)

- Per-provider `Error` state. Open follow-up issue: "Per-provider error attribution for Memory Tree" — proposes either parsing `payload_json` for failed jobs to extract `source_id`, or adding a typed `source_kind` column to `mem_tree_jobs` (probably the latter, with a one-shot migration).
