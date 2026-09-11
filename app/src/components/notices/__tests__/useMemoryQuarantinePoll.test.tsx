import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MEMORY_QUARANTINE_POLL_MS, useMemoryQuarantinePoll } from '../useMemoryQuarantinePoll';

const mockStatus = vi.fn();
const mockDispatch = vi.fn();
let mockAuthenticated = true;

vi.mock('../../../utils/tauriCommands/memoryTree', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../utils/tauriCommands/memoryTree')>();
  return { ...actual, memoryTreePipelineStatus: (...args: unknown[]) => mockStatus(...args) };
});
vi.mock('../../../store/hooks', () => ({ useAppDispatch: () => mockDispatch }));
vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({ snapshot: { auth: { isAuthenticated: mockAuthenticated } } }),
}));

function statusWith(quarantine: unknown) {
  return {
    status: 'idle',
    reason: null,
    last_sync_ms: 0,
    total_chunks: 0,
    wiki_size_bytes: 0,
    pipeline_jobs: { ready: 0, running: 0, failed: 0, done: 0 },
    is_syncing: false,
    is_paused: false,
    quarantine,
  };
}

describe('useMemoryQuarantinePoll', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockStatus.mockReset();
    mockDispatch.mockReset();
    mockAuthenticated = true;
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('reports an outstanding quarantine on mount and again on the interval', async () => {
    mockStatus.mockResolvedValue(
      statusWith({ quarantined_at_ms: 1, quarantined_path: '/x', resynced: false })
    );
    renderHook(() => useMemoryQuarantinePoll());
    await act(async () => {
      await Promise.resolve();
    });
    expect(mockStatus).toHaveBeenCalledTimes(1);
    expect(mockDispatch).toHaveBeenCalledWith(
      expect.objectContaining({
        type: 'userErrors/reportUserError',
        payload: expect.objectContaining({
          descriptor: expect.objectContaining({ kind: 'memory_store_corrupt' }),
        }),
      })
    );

    await act(async () => {
      vi.advanceTimersByTime(MEMORY_QUARANTINE_POLL_MS);
      await Promise.resolve();
    });
    expect(mockStatus).toHaveBeenCalledTimes(2);
  });

  it('resolves the notice once the store has been re-synced', async () => {
    mockStatus.mockResolvedValue(
      statusWith({ quarantined_at_ms: 1, quarantined_path: '/x', resynced: true })
    );
    renderHook(() => useMemoryQuarantinePoll());
    await act(async () => {
      await Promise.resolve();
    });
    expect(mockDispatch).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'userErrors/resolveUserError' })
    );
  });

  it('does nothing while signed out, and swallows a failed status read', async () => {
    mockAuthenticated = false;
    renderHook(() => useMemoryQuarantinePoll());
    await act(async () => {
      await Promise.resolve();
    });
    expect(mockStatus).not.toHaveBeenCalled();

    mockAuthenticated = true;
    mockStatus.mockRejectedValue(new Error('core restarting'));
    renderHook(() => useMemoryQuarantinePoll());
    await act(async () => {
      await Promise.resolve();
    });
    expect(mockDispatch).not.toHaveBeenCalled();
  });
});
