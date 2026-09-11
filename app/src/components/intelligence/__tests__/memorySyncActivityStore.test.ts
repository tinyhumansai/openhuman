/**
 * @vitest-environment jsdom
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SourceStatus } from '../../../services/memorySourcesService';
import {
  applyStageEvent,
  getMemorySyncActivity,
  noteSyncRejected,
  noteSyncRequested,
  RECONCILE_GRACE_MS,
  reconcileWithStatuses,
  resetMemorySyncActivityForTests,
  subscribeTerminalSyncEvents,
} from '../memorySyncActivityStore';

function status(source_id: string, extra: Partial<SourceStatus> = {}): SourceStatus {
  return {
    source_id,
    chunks_synced: 0,
    chunks_pending: 0,
    last_chunk_at_ms: null,
    freshness: 'idle',
    ...extra,
  };
}

describe('memorySyncActivityStore', () => {
  beforeEach(() => {
    resetMemorySyncActivityForTests();
  });

  it('keeps a live bar per row from the stage stream until the run ends', () => {
    applyStageEvent({ stage: 'running', source_id: 'src-a', detail: null });
    let s = getMemorySyncActivity();
    expect(s.progress.get('src-a')).toEqual({ stage: 'running', detail: null, percent: null });

    const ended = vi.fn();
    const unsubscribe = subscribeTerminalSyncEvents(ended);
    applyStageEvent({ stage: 'completed', source_id: 'src-a', detail: 'ingested 7 item(s)' });
    s = getMemorySyncActivity();
    expect(s.progress.has('src-a')).toBe(false);
    expect(s.syncingIds.has('src-a')).toBe(false);
    expect(s.results.get('src-a')).toEqual({ kind: 'success', items: 7, reason: null, note: null });
    expect(ended).toHaveBeenCalledWith(
      expect.objectContaining({
        rowId: 'src-a',
        stage: 'completed',
        result: { kind: 'success', items: 7, reason: null, note: null },
      })
    );
    unsubscribe();
  });

  it('is fed by the window event without any screen mounted', () => {
    window.dispatchEvent(
      new CustomEvent('openhuman:memory-sync-stage', {
        detail: { stage: 'fetching', source_id: 'src-b', detail: '2/4 pages' },
      })
    );
    const s = getMemorySyncActivity();
    expect(s.progress.get('src-b')).toEqual({
      stage: 'fetching',
      detail: '2/4 pages',
      percent: 50,
    });
    expect(s.syncingIds.has('src-b')).toBe(true);
  });

  it('lights the row on request and records a rejected request as a failed result', () => {
    applyStageEvent({ stage: 'completed', source_id: 'src-c', detail: 'ingested 1 item(s)' });
    noteSyncRequested('src-c');
    let s = getMemorySyncActivity();
    expect(s.syncingIds.has('src-c')).toBe(true);
    expect(s.results.has('src-c')).toBe(false);

    noteSyncRejected('src-c', 'transport down');
    s = getMemorySyncActivity();
    expect(s.syncingIds.has('src-c')).toBe(false);
    expect(s.results.get('src-c')).toEqual({
      kind: 'failed',
      items: null,
      reason: 'transport down',
      note: null,
    });
  });

  it('seeds a run the core reports that the store did not see start', () => {
    reconcileWithStatuses([status('src-d', { sync_stage: 'running', sync_detail: 'pass 2' })]);
    const s = getMemorySyncActivity();
    expect(s.progress.get('src-d')).toEqual({ stage: 'running', detail: 'pass 2', percent: null });
    // The flag follows the bar on a cold mount: the button must read as
    // syncing, not only the row.
    expect(s.syncingIds.has('src-d')).toBe(true);
  });

  it("marks the connector's `running` stage as syncing like the reader stages", () => {
    applyStageEvent({ stage: 'running', source_id: 'src-h', detail: null });
    expect(getMemorySyncActivity().syncingIds.has('src-h')).toBe(true);
  });

  it('leaves a fresh local entry alone when the core says idle, and clears a stale one', () => {
    const t0 = 1_000_000;
    applyStageEvent({ stage: 'running', source_id: 'src-e', detail: null });
    // The store stamps Date.now(); reconcile with a "now" inside the grace.
    reconcileWithStatuses([status('src-e', { sync_stage: null, sync_detail: null })], Date.now());
    expect(getMemorySyncActivity().progress.has('src-e')).toBe(true);

    reconcileWithStatuses(
      [status('src-e', { sync_stage: null, sync_detail: null })],
      Date.now() + RECONCILE_GRACE_MS + t0
    );
    expect(getMemorySyncActivity().progress.has('src-e')).toBe(false);
    expect(getMemorySyncActivity().syncingIds.has('src-e')).toBe(false);
  });

  it('seeds the bar for a row the button lit when the poll reports it live', () => {
    // The optimistic flag has no stage; if the first socket event was missed,
    // the poll carries it.
    noteSyncRequested('src-g');
    reconcileWithStatuses([status('src-g', { sync_stage: 'running', sync_detail: null })]);
    const s = getMemorySyncActivity();
    expect(s.progress.get('src-g')).toEqual({ stage: 'running', detail: null, percent: null });
    expect(s.syncingIds.has('src-g')).toBe(true);
  });

  it('changes nothing for a core that does not report the field', () => {
    applyStageEvent({ stage: 'running', source_id: 'src-f', detail: null });
    reconcileWithStatuses([status('src-f')], Date.now() + RECONCILE_GRACE_MS * 10);
    expect(getMemorySyncActivity().progress.has('src-f')).toBe(true);
  });
});
