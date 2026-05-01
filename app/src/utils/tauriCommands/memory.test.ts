/**
 * Unit tests for memory RPC wrappers: memorySyncChannel, memorySyncAll, memoryLearnAll.
 */
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri } from './common';
import { memoryLearnAll, memorySyncAll, memorySyncChannel } from './memory';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('./common', () => ({ isTauri: vi.fn(() => true) }));

const mockCallCoreRpc = callCoreRpc as Mock;
const mockIsTauri = isTauri as Mock;

beforeEach(() => {
  vi.clearAllMocks();
  mockIsTauri.mockReturnValue(true);
});

describe('memorySyncChannel', () => {
  test('throws when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(memorySyncChannel('ch-1')).rejects.toThrow('Not running in Tauri');
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });

  test('calls memory_sync_channel with correct channel_id', async () => {
    const mockResp = { requested: true, channel_id: 'ch-1' };
    mockCallCoreRpc.mockResolvedValueOnce(mockResp);

    const result = await memorySyncChannel('ch-1');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_sync_channel',
      params: { channel_id: 'ch-1' },
    });
    expect(result).toEqual(mockResp);
  });
});

describe('memorySyncAll', () => {
  test('throws when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(memorySyncAll()).rejects.toThrow('Not running in Tauri');
  });

  test('calls memory_sync_all and returns result', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ requested: true });

    const result = await memorySyncAll();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.memory_sync_all' });
    expect(result).toEqual({ requested: true });
  });
});

describe('memoryLearnAll', () => {
  test('throws when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(memoryLearnAll()).rejects.toThrow('Not running in Tauri');
  });

  test('calls memory_learn_all without namespaces param when none provided', async () => {
    const mockResp = { namespaces_processed: 0, results: [] };
    mockCallCoreRpc.mockResolvedValueOnce(mockResp);

    const result = await memoryLearnAll();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_learn_all',
      params: {},
    });
    expect(result.namespaces_processed).toBe(0);
  });

  test('includes namespaces param when provided', async () => {
    const mockResp = {
      namespaces_processed: 1,
      results: [{ namespace: 'research', status: 'ok' }],
    };
    mockCallCoreRpc.mockResolvedValueOnce(mockResp);

    const result = await memoryLearnAll(['research']);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_learn_all',
      params: { namespaces: ['research'] },
    });
    expect(result.namespaces_processed).toBe(1);
    expect(result.results[0].status).toBe('ok');
  });

  test('returns aggregated error results without throwing', async () => {
    const mockResp = {
      namespaces_processed: 2,
      results: [
        { namespace: 'ns-a', status: 'ok' },
        { namespace: 'ns-b', status: 'error', error: 'local AI not enabled' },
      ],
    };
    mockCallCoreRpc.mockResolvedValueOnce(mockResp);

    const result = await memoryLearnAll();

    expect(result.namespaces_processed).toBe(2);
    const errEntry = result.results.find(r => r.status === 'error');
    expect(errEntry?.namespace).toBe('ns-b');
    expect(errEntry?.error).toContain('local AI');
  });
});
