import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it, vi } from 'vitest';

import reducer, { fetchAndHydrateTurnHistory, fetchAndHydrateTurnState } from '../chatRuntimeSlice';

const { mockThreadApi } = vi.hoisted(() => ({
  mockThreadApi: { getTurnState: vi.fn(), listRuns: vi.fn(), getTurnStateHistory: vi.fn() },
}));

vi.mock('../../services/api/threadApi', () => ({ threadApi: mockThreadApi }));

describe('fetchAndHydrateTurnState', () => {
  it('hydrates durable run ledger rows when no live turn snapshot exists', async () => {
    const store = configureStore({ reducer });
    mockThreadApi.getTurnState.mockResolvedValueOnce(null);
    mockThreadApi.listRuns.mockResolvedValueOnce([
      {
        id: 'sub-run-1',
        kind: 'subagent',
        parentThreadId: 'thread-runs',
        agentId: 'researcher',
        status: 'completed',
        metadata: {},
        startedAt: '2026-06-04T12:00:00Z',
        updatedAt: '2026-06-04T12:00:04Z',
      },
    ]);

    await store.dispatch(fetchAndHydrateTurnState('thread-runs'));

    expect(mockThreadApi.listRuns).toHaveBeenCalledWith({
      parentThreadId: 'thread-runs',
      limit: 50,
    });
    expect(store.getState().toolTimelineByThread['thread-runs']).toEqual([
      expect.objectContaining({
        id: 'subagent:sub-run-1',
        status: 'success',
        sourceToolName: 'run_ledger',
      }),
    ]);
  });
});

describe('fetchAndHydrateTurnHistory', () => {
  const persistedTurn = (
    requestId: string,
    lifecycle: string,
    startedAt: string,
    tools: number,
    transcript?: Array<Record<string, unknown>>
  ) => ({
    threadId: 'thread-hist',
    requestId,
    lifecycle,
    iteration: 1,
    maxIterations: 25,
    streamingText: '',
    thinking: '',
    toolTimeline: Array.from({ length: tools }, (_, i) => ({
      id: `${requestId}-tc-${i}`,
      name: 'shell',
      round: 0,
      status: 'success' as const,
    })),
    ...(transcript ? { transcript } : {}),
    startedAt,
    updatedAt: startedAt,
  });

  it('stores older settled turns by requestId, skipping the latest and the live/started turn', async () => {
    const store = configureStore({ reducer });
    // Newest-first: req-3 (latest, skipped), req-2 (completed, kept),
    // req-1 (interrupted, kept), req-0 (started → filtered by lifecycle).
    mockThreadApi.getTurnStateHistory.mockResolvedValueOnce([
      persistedTurn('req-3', 'completed', '2026-06-04T13:00:00Z', 1),
      persistedTurn('req-2', 'completed', '2026-06-04T12:00:00Z', 2),
      persistedTurn('req-1', 'interrupted', '2026-06-04T11:00:00Z', 1),
      persistedTurn('req-0', 'started', '2026-06-04T10:00:00Z', 1),
    ]);

    await store.dispatch(fetchAndHydrateTurnHistory('thread-hist'));

    const timelines = store.getState().turnTimelinesByThread['thread-hist'];
    expect(Object.keys(timelines).sort()).toEqual(['req-1', 'req-2']);
    expect(timelines['req-2']).toHaveLength(2);
    expect(timelines['req-1']).toHaveLength(1);
    expect(timelines['req-3']).toBeUndefined();
    expect(timelines['req-0']).toBeUndefined();
  });

  it("keeps each past turn's reasoning/narration transcript (fix 1), ordered by seq (fix 5)", async () => {
    const store = configureStore({ reducer });
    mockThreadApi.getTurnStateHistory.mockResolvedValueOnce([
      // req-latest is skipped (index 0).
      persistedTurn('req-latest', 'completed', '2026-06-04T13:00:00Z', 1),
      // req-a: has tools AND a transcript, deliberately out of seq order.
      persistedTurn('req-a', 'completed', '2026-06-04T12:00:00Z', 1, [
        { kind: 'narration', round: 0, seq: 1, text: 'second' },
        { kind: 'thinking', round: 0, seq: 0, text: 'first' },
      ]),
      // req-b: a tool-LESS turn that only thought/narrated — must still be kept
      // for its transcript rather than dropped.
      persistedTurn('req-b', 'completed', '2026-06-04T11:00:00Z', 0, [
        { kind: 'thinking', round: 0, seq: 0, text: 'only thinking, no tools' },
      ]),
    ]);

    await store.dispatch(fetchAndHydrateTurnHistory('thread-hist'));

    const transcripts = store.getState().turnTranscriptsByThread['thread-hist'];
    expect(Object.keys(transcripts).sort()).toEqual(['req-a', 'req-b']);
    // Ordered by persisted `seq`, not wire order.
    expect(transcripts['req-a'].map(i => ('text' in i ? i.text : undefined))).toEqual([
      'first',
      'second',
    ]);
    // The tool-less turn is retained purely for its transcript.
    const timelines = store.getState().turnTimelinesByThread['thread-hist'];
    expect(timelines['req-b']).toBeUndefined();
    expect(transcripts['req-b']).toHaveLength(1);
  });

  it('swallows transport failures without throwing', async () => {
    const store = configureStore({ reducer });
    mockThreadApi.getTurnStateHistory.mockRejectedValueOnce(new Error('boom'));
    await expect(store.dispatch(fetchAndHydrateTurnHistory('t'))).resolves.toBeDefined();
    expect(store.getState().turnTimelinesByThread['t']).toBeUndefined();
    expect(store.getState().turnTranscriptsByThread['t']).toBeUndefined();
  });
});
