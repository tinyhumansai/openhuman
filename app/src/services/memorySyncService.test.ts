import { beforeEach, describe, expect, it, vi } from 'vitest';

import { type MemorySyncStatus, memorySyncStatusList } from './memorySyncService';

const mockCallCoreRpc = vi.fn();

vi.mock('./coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

function makeStatus(overrides: Partial<MemorySyncStatus> = {}): MemorySyncStatus {
  return {
    provider: 'gmail',
    chunks_synced: 0,
    chunks_pending: 0,
    batch_total: 0,
    batch_processed: 0,
    last_chunk_at_ms: null,
    freshness: 'idle',
    ...overrides,
  };
}

describe('memorySyncService.memorySyncStatusList', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls the correct RPC method without params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ statuses: [] });
    await memorySyncStatusList();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.memory_sync_status_list' });
  });

  it('returns the statuses array from the envelope', async () => {
    const status = makeStatus({ provider: 'slack', chunks_synced: 42, freshness: 'active' });
    mockCallCoreRpc.mockResolvedValueOnce({ statuses: [status] });
    const out = await memorySyncStatusList();
    expect(out).toEqual([status]);
  });

  it('propagates RPC errors as thrown errors', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('rpc boom'));
    await expect(memorySyncStatusList()).rejects.toThrow('rpc boom');
  });

  it('returns empty array on malformed response (missing statuses[])', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ wrong: 'shape' });
    const out = await memorySyncStatusList();
    expect(out).toEqual([]);
  });

  it('returns empty array on null response', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(null);
    const out = await memorySyncStatusList();
    expect(out).toEqual([]);
  });
});
