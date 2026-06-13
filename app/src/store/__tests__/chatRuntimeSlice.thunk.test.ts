import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it, vi } from 'vitest';

import reducer, { fetchAndHydrateTurnState } from '../chatRuntimeSlice';

const { mockThreadApi } = vi.hoisted(() => ({
  mockThreadApi: { getTurnState: vi.fn(), listRuns: vi.fn() },
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
