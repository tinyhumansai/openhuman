import { describe, expect, it, vi } from 'vitest';

import { providerSurfacesApi } from '../../services/api/providerSurfacesApi';
import reducer, { fetchRespondQueue } from '../providerSurfaceSlice';

vi.mock('../../services/api/providerSurfacesApi', () => ({
  providerSurfacesApi: { listQueue: vi.fn() },
}));

describe('providerSurfaceSlice', () => {
  it('stores queue payload on success', async () => {
    vi.mocked(providerSurfacesApi.listQueue).mockResolvedValue({
      items: [
        {
          id: 'linkedin:acct-1:message:entity-1',
          provider: 'linkedin',
          accountId: 'acct-1',
          eventKind: 'message',
          entityId: 'entity-1',
          timestamp: '2026-04-22T17:00:00Z',
          requiresAttention: true,
          status: 'pending',
        },
      ],
      count: 1,
    });

    const pending = reducer(undefined, fetchRespondQueue.pending('', undefined));
    expect(pending.status).toBe('loading');

    const fulfilledAction = fetchRespondQueue.fulfilled(
      await providerSurfacesApi.listQueue(),
      '',
      undefined
    );
    const state = reducer(pending, fulfilledAction);
    expect(state.status).toBe('succeeded');
    expect(state.count).toBe(1);
    expect(state.queue[0]?.provider).toBe('linkedin');
    expect(state.lastSyncedAt).not.toBeNull();
  });

  it('stores rejected message on failure', () => {
    const state = reducer(
      undefined,
      fetchRespondQueue.rejected(new Error('boom'), '', undefined, 'boom')
    );
    expect(state.status).toBe('failed');
    expect(state.error).toBe('boom');
  });

  it('silent pending does not switch status to loading', () => {
    const state = reducer(undefined, fetchRespondQueue.pending('', { silent: true }));
    expect(state.status).toBe('idle');
  });

  it('silent rejection does not change status', () => {
    const state = reducer(
      undefined,
      fetchRespondQueue.rejected(new Error('network'), '', { silent: true }, 'network')
    );
    expect(state.status).toBe('idle');
    expect(state.error).toBeNull();
  });
});
