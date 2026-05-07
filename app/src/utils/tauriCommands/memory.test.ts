/**
 * Unit tests for memory RPC wrappers: memorySyncChannel, memorySyncAll, memoryLearnAll.
 */
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri } from './common';
import {
  aiListMemoryFiles,
  memoryLearnAll,
  memorySyncAll,
  memorySyncChannel,
  whatsappListChats,
  whatsappListMessages,
} from './memory';

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

describe('aiListMemoryFiles', () => {
  test('defaults relative_dir to empty string (list memory root)', async () => {
    // Regression guard: the wrapper used to default to 'memory', and
    // the Rust resolver joined that onto `<workspace>/memory/`,
    // producing the doomed `<workspace>/memory/memory` path. Empty
    // string is the resolver's "the memory root" sentinel.
    mockCallCoreRpc.mockResolvedValueOnce({ data: { files: ['a.md', 'b.md'] } });

    const files = await aiListMemoryFiles();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_list_files',
      params: { relative_dir: '' },
    });
    expect(files).toEqual(['a.md', 'b.md']);
  });

  test('forwards an explicit relativeDir verbatim', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ files: ['nested.md'] });
    const files = await aiListMemoryFiles('subdir');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_list_files',
      params: { relative_dir: 'subdir' },
    });
    expect(files).toEqual(['nested.md']);
  });

  test('returns [] when the response has no recognisable files array', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(null);
    expect(await aiListMemoryFiles()).toEqual([]);

    mockCallCoreRpc.mockResolvedValueOnce({ unrelated: 'shape' });
    expect(await aiListMemoryFiles()).toEqual([]);
  });

  test('throws when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(aiListMemoryFiles()).rejects.toThrow(/Not running in Tauri/);
  });
});

describe('whatsappListChats', () => {
  test('throws when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(whatsappListChats()).rejects.toThrow('Not running in Tauri');
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });

  test('calls correct RPC method with provided params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);
    await whatsappListChats({ limit: 10 });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.whatsapp_data_list_chats',
      params: { limit: 10 },
    });
  });

  test('uses empty params object when none provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);
    await whatsappListChats();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.whatsapp_data_list_chats',
      params: {},
    });
  });

  test('returns array directly when response is already an array', async () => {
    const chats = [{ chat_id: 'c1', display_name: 'Direct' }];
    mockCallCoreRpc.mockResolvedValueOnce(chats);
    const result = await whatsappListChats();
    expect(result).toBe(chats);
  });

  test('extracts result field from wrapped response envelope', async () => {
    const chats = [{ chat_id: 'c2', display_name: 'Wrapped' }];
    mockCallCoreRpc.mockResolvedValueOnce({ result: chats, logs: [] });
    const result = await whatsappListChats();
    expect(result).toEqual(chats);
  });

  test('returns empty array when wrapped response has no result field', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ logs: [] });
    const result = await whatsappListChats();
    expect(result).toEqual([]);
  });
});

describe('whatsappListMessages', () => {
  test('throws when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(whatsappListMessages({ chat_id: 'c1' })).rejects.toThrow('Not running in Tauri');
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });

  test('calls correct RPC method with required chat_id param', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);
    await whatsappListMessages({ chat_id: 'c1', limit: 50 });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.whatsapp_data_list_messages',
      params: { chat_id: 'c1', limit: 50 },
    });
  });

  test('returns array directly when response is already an array', async () => {
    const msgs = [{ message_id: 'm1', body: 'hello' }];
    mockCallCoreRpc.mockResolvedValueOnce(msgs);
    const result = await whatsappListMessages({ chat_id: 'c1' });
    expect(result).toBe(msgs);
  });

  test('extracts result field from wrapped response envelope', async () => {
    const msgs = [{ message_id: 'm2', body: 'world' }];
    mockCallCoreRpc.mockResolvedValueOnce({ result: msgs, logs: [] });
    const result = await whatsappListMessages({ chat_id: 'c1' });
    expect(result).toEqual(msgs);
  });

  test('returns empty array when wrapped response has no result field', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ logs: [] });
    const result = await whatsappListMessages({ chat_id: 'c1' });
    expect(result).toEqual([]);
  });
});
