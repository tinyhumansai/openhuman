import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useMemoryIngestionStatus } from '../useMemoryIngestionStatus';

const mockCallCoreRpc = vi.fn();

vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (args: unknown) => mockCallCoreRpc(args),
}));

describe('useMemoryIngestionStatus', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('maps the snake_case RPC envelope into camelCase status', async () => {
    mockCallCoreRpc.mockResolvedValue({
      running: true,
      current_document_id: 'doc-1',
      current_title: 'Notes',
      current_namespace: 'global',
      queue_depth: 2,
      last_completed_at: 1700000000000,
      last_document_id: 'doc-0',
      last_success: true,
    });

    const { result } = renderHook(() => useMemoryIngestionStatus());

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.status).toEqual({
      running: true,
      currentDocumentId: 'doc-1',
      currentTitle: 'Notes',
      currentNamespace: 'global',
      queueDepth: 2,
      lastCompletedAt: 1700000000000,
      lastDocumentId: 'doc-0',
      lastSuccess: true,
    });
    expect(result.current.error).toBeNull();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.memory_ingestion_status' });
  });

  it('reports an error when the RPC fails and keeps idle defaults', async () => {
    mockCallCoreRpc.mockRejectedValue(new Error('boom'));

    const { result } = renderHook(() => useMemoryIngestionStatus());

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.status.running).toBe(false);
    expect(result.current.status.queueDepth).toBe(0);
    expect(result.current.error).toBe('boom');
  });

  it('refresh() re-issues the RPC and updates status', async () => {
    mockCallCoreRpc
      .mockResolvedValueOnce({ running: false, queue_depth: 0 })
      .mockResolvedValueOnce({ running: true, queue_depth: 1, current_document_id: 'doc-2' });

    const { result } = renderHook(() => useMemoryIngestionStatus());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.status.running).toBe(false);

    await act(async () => {
      await result.current.refresh();
    });

    expect(result.current.status.running).toBe(true);
    expect(result.current.status.queueDepth).toBe(1);
    expect(result.current.status.currentDocumentId).toBe('doc-2');
  });
});
