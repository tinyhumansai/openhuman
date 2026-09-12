/**
 * Live per-source sync state that outlives the Sources screen (openhuman#6019).
 *
 * The core runs a sync in a detached task and narrates it over the socket
 * (`memory:sync_stage` → `openhuman:memory-sync-stage` on `window`). The
 * Sources screen used to keep the resulting state — which rows are syncing,
 * their live bar, their last result — in component state, with the window
 * listener registered in an effect. Both went away with the component: a tab
 * or route change mid-sync dropped the bar, the terminal event landed on
 * nobody, and the row came back idle as if the sync had stopped. It had not.
 *
 * This module is the state's home instead. The listener is installed once,
 * for the life of the page, and screens read the store; a screen that mounts
 * mid-run finds the run where it is. What a screen still owns is the toast
 * for a run that ends while it is mounted — see `subscribeTerminalSyncEvents`.
 *
 * The store is also fed from the other direction: `memory_sources.status_list`
 * now says which sources the core has in flight, so a cold mount (or an app
 * reload mid-run) seeds the bar from the poll, and a bar whose terminal event
 * was lost is taken down once the core reports the row idle.
 */
import { useSyncExternalStore } from 'react';

import type { SourceStatus } from '../../services/memorySourcesService';
import {
  STAGE_FALLBACK_PERCENT,
  type SyncNote,
  type SyncProgress,
  type SyncResult,
} from './memorySourcesSyncTypes';

/**
 * Parse a sync progress detail string into a 0–100 percent.
 *
 * - Recognises "N/M ..." numeric patterns and returns N/M as a ratio.
 * - Falls back to the per-stage baseline when no ratio is present rather
 *   than returning a bogus number (RC#4, issue #3295).
 * - Returns `null` when both approaches are unavailable (no stage either).
 */
export function parseSyncProgress(detail: string | null, stage?: string): number | null {
  // Try the numeric "N/M ..." ratio first.
  if (detail) {
    const match = detail.match(/^(\d+)\/(\d+)[\s/]/);
    if (match) {
      const current = parseInt(match[1], 10);
      const total = parseInt(match[2], 10);
      if (total > 0) return Math.round((current / total) * 100);
    }
  }
  // Fall back to the per-stage baseline percentage.
  if (stage && stage in STAGE_FALLBACK_PERCENT) {
    return STAGE_FALLBACK_PERCENT[stage];
  }
  return null;
}

/**
 * Parse the number of newly-ingested items from a `completed` stage detail
 * string. The backend formats this as `"ingested N item(s)"`
 * (`memory_sources/sync.rs`). Returns `null` when no count is present so the
 * UI can fall back to a generic "synced" confirmation (#3295).
 */
export function parseIngestedCount(detail: string | null): number | null {
  if (!detail) return null;
  const match = detail.match(/ingested\s+(\d+)\s+item/i);
  if (match) return parseInt(match[1], 10);
  return null;
}

/**
 * Why a completed run stopped short, from the remainder the core writes after
 * the count: `", more pending — Sync again to continue"` when the per-run cap
 * left more to read, `"; today's provider request budget is spent"` when the
 * day's budget did. The number alone made both read as a finished sync, and a
 * spent budget with zero new items read as "Up to date" — the opposite of what
 * happened. The budget wins when both appear: it is the reason nothing more
 * will arrive today.
 */
export function parseSyncNote(detail: string | null): SyncNote | null {
  if (!detail) return null;
  const lower = detail.toLowerCase();
  if (lower.includes('budget')) return 'budget_spent';
  if (lower.includes('more pending')) return 'more_pending';
  return null;
}

/** The store's snapshot: one immutable object, replaced on every change. */
export interface MemorySyncActivity {
  /** Rows a sync is running for — the button's spinner and disabled state. */
  syncingIds: ReadonlySet<string>;
  /** Live bar per row while a run is in flight. */
  progress: ReadonlyMap<string, SyncProgress>;
  /** Terminal chip per row after a run ended, until the next run starts. */
  results: ReadonlyMap<string, SyncResult>;
}

/** The `openhuman:memory-sync-stage` event's `detail` payload. */
export interface SyncStageEventDetail {
  stage?: string;
  /** Originating memory-source id (RC#2, #3295). Preferred over connection_id. */
  source_id?: string | null;
  /** Legacy: document/connection id. Still present for backward compat. */
  connection_id?: string | null;
  detail?: string | null;
}

/** A run ending, for the screen that wants to say so. */
export interface TerminalSyncEvent {
  rowId: string;
  stage: 'completed' | 'failed';
  detail: string | null;
  result: SyncResult;
}

type TerminalListener = (event: TerminalSyncEvent) => void;

/**
 * A local live entry younger than this is not overruled by a poll that says
 * idle: the poll's answer may predate the `running` stage, and the button's
 * optimistic flag always predates the core's first stage.
 */
export const RECONCILE_GRACE_MS = 10_000;

/**
 * Stages that mean a run is in progress for the button's sake (RC#1, #3295).
 * `running` is the connector's one live stage; the reader path narrates the
 * rest.
 */
const SYNCING_STAGES = new Set([
  'running',
  'requested',
  'fetching',
  'stored',
  'queued',
  'ingesting',
]);

const EMPTY: MemorySyncActivity = {
  syncingIds: new Set(),
  progress: new Map(),
  results: new Map(),
};

let state: MemorySyncActivity = EMPTY;
/** When each row's live entry was last written — the reconcile grace clock. */
const liveSince = new Map<string, number>();
const listeners = new Set<() => void>();
const terminalListeners = new Set<TerminalListener>();

function commit(next: MemorySyncActivity): void {
  state = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): MemorySyncActivity {
  return state;
}

/** The live state, re-rendering on every change. */
export function useMemorySyncActivity(): MemorySyncActivity {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

/** The live state, once. */
export function getMemorySyncActivity(): MemorySyncActivity {
  return state;
}

/**
 * Be told when a run ends. The store records the result either way; this is
 * for the screen-level courtesy of a toast, which only makes sense while a
 * screen is there to show it.
 */
export function subscribeTerminalSyncEvents(listener: TerminalListener): () => void {
  terminalListeners.add(listener);
  return () => {
    terminalListeners.delete(listener);
  };
}

/** Apply one stage event. Exported for tests; the window listener below is the caller. */
export function applyStageEvent(data: SyncStageEventDetail | null | undefined): void {
  // RC#2 (#3295): prefer source_id when present; fall back to connection_id for
  // backward compat with older core versions that don't emit source_id yet.
  const rowId = data?.source_id ?? data?.connection_id;
  if (!rowId) return;
  const stage = data?.stage ?? '';
  const detail = data?.detail ?? null;

  console.debug(
    `[ui-flow][memory-sync] stage=${stage} rowId=${rowId} source_id=${data?.source_id ?? 'absent'} connection_id=${data?.connection_id ?? 'absent'}`
  );

  if (stage === 'completed' || stage === 'failed') {
    // Success: the item count parsed from the detail ("ingested N item(s)")
    // and why the run stopped short, if it did; 0 new items → "up to date"
    // (#3295). Failure: the reason, verbatim.
    const result: SyncResult =
      stage === 'completed'
        ? {
            kind: 'success',
            items: parseIngestedCount(detail),
            reason: null,
            note: parseSyncNote(detail),
          }
        : { kind: 'failed', items: null, reason: detail, note: null };
    const progress = new Map(state.progress);
    progress.delete(rowId);
    liveSince.delete(rowId);
    const syncingIds = new Set(state.syncingIds);
    syncingIds.delete(rowId);
    const results = new Map(state.results);
    results.set(rowId, result);
    commit({ syncingIds, progress, results });
    for (const listener of terminalListeners) listener({ rowId, stage, detail, result });
    return;
  }

  // Non-terminal stage: a sync is genuinely in progress. Drop any stale
  // terminal result for this row so the live bar replaces the old chip.
  const results = new Map(state.results);
  results.delete(rowId);
  const progress = new Map(state.progress);
  progress.set(rowId, { stage, detail, percent: parseSyncProgress(detail, stage) });
  liveSince.set(rowId, Date.now());
  const syncingIds = new Set(state.syncingIds);
  if (SYNCING_STAGES.has(stage)) syncingIds.add(rowId);
  commit({ syncingIds, progress, results });
}

/**
 * The Sync button was pressed: light the row before the first stage arrives
 * (RC#1, #3295) and drop the previous run's chip.
 */
export function noteSyncRequested(rowId: string): void {
  const syncingIds = new Set(state.syncingIds);
  syncingIds.add(rowId);
  const results = new Map(state.results);
  results.delete(rowId);
  liveSince.set(rowId, Date.now());
  commit({ syncingIds, progress: state.progress, results });
}

/**
 * The sync RPC itself was rejected, so no run started and no stage event will
 * come: take the row down and record the reason as its result.
 */
export function noteSyncRejected(rowId: string, reason: string): void {
  const syncingIds = new Set(state.syncingIds);
  syncingIds.delete(rowId);
  const progress = new Map(state.progress);
  progress.delete(rowId);
  liveSince.delete(rowId);
  const results = new Map(state.results);
  results.set(rowId, { kind: 'failed', items: null, reason, note: null });
  commit({ syncingIds, progress, results });
}

/**
 * Fold the core's own account of what is in flight into the store.
 *
 * A row the core reports running that the store knows nothing about gets its
 * bar (a cold mount, an app reload mid-run). A row the store has live that the
 * core reports idle is taken down once its entry is older than the grace —
 * the terminal event was lost, or never named the row. Rows from a core that
 * does not report the field (`sync_stage` absent, not `null`) say nothing
 * and change nothing.
 */
export function reconcileWithStatuses(statuses: SourceStatus[], now = Date.now()): void {
  const syncingIds = new Set(state.syncingIds);
  const progress = new Map(state.progress);
  const results = new Map(state.results);
  let changed = false;
  for (const status of statuses) {
    if (status.sync_stage === undefined) continue;
    const rowId = status.source_id;
    const live = status.sync_stage;
    const knownLive = progress.has(rowId) || syncingIds.has(rowId);
    if (live) {
      // The core has the row running: the flag follows, whether the store
      // learns of the run here (a cold mount) or already had the bar.
      if (!syncingIds.has(rowId)) {
        syncingIds.add(rowId);
        changed = true;
      }
      // A row the button lit optimistically has no bar yet; if its first
      // stage event was missed, the poll is what carries the stage.
      if (progress.has(rowId)) continue;
      const detail = status.sync_detail ?? null;
      progress.set(rowId, { stage: live, detail, percent: parseSyncProgress(detail, live) });
      liveSince.set(rowId, now);
      results.delete(rowId);
      changed = true;
    } else if (knownLive) {
      if (now - (liveSince.get(rowId) ?? 0) < RECONCILE_GRACE_MS) continue;
      progress.delete(rowId);
      liveSince.delete(rowId);
      syncingIds.delete(rowId);
      changed = true;
    }
  }
  if (changed) commit({ syncingIds, progress, results });
}

/** Back to nothing in flight. Tests only. */
export function resetMemorySyncActivityForTests(): void {
  liveSince.clear();
  commit(EMPTY);
}

// Installed once, for the life of the page, and never removed: the socket
// service dispatches the event globally whether or not any screen is showing,
// and the whole point is to be listening while none is.
if (typeof window !== 'undefined') {
  window.addEventListener('openhuman:memory-sync-stage', event => {
    applyStageEvent((event as CustomEvent<SyncStageEventDetail | null>).detail);
  });
}
