import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import {
  isMemoryOn,
  memoryContextGet,
  memoryContextRefresh,
  memoryContextSet,
  memoryConversationsGet,
  memoryConversationsSet,
  memoryEngineGet,
  memoryEngineSet,
  memoryEnginesList,
  memoryErrorCode,
  memoryErrorMessage,
  memoryFetch,
  memoryForget,
  memoryImportScan,
  memoryImportStart,
  memoryImportStatus,
  memoryItemsList,
  memoryLearn,
  memoryRecall,
  memorySourcesAdd,
  memorySourcesList,
  memorySourcesRemove,
  memorySourcesSync,
} from './memoryApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const rpc = vi.mocked(callCoreRpc);

beforeEach(() => {
  rpc.mockReset();
  rpc.mockResolvedValue({});
});

describe('memoryApi wire calls', () => {
  // [call, expected method, expected params]
  const cases: Array<[string, () => Promise<unknown>, string, Record<string, unknown>]> = [
    ['engines list', () => memoryEnginesList(), 'openhuman.memory_engines_list', {}],
    ['engine get', () => memoryEngineGet(), 'openhuman.memory_engine_get', {}],
    [
      'engine set',
      () => memoryEngineSet({ engine: 'cortexdb', endpoint: 'https://x', api_key: 'k' }),
      'openhuman.memory_engine_set',
      { engine: 'cortexdb', endpoint: 'https://x', api_key: 'k' },
    ],
    [
      'recall',
      () => memoryRecall({ question: 'q', filter: { kinds: ['learning'] } }),
      'openhuman.memory_recall',
      { question: 'q', filter: { kinds: ['learning'] } },
    ],
    [
      'fetch drops undefined params',
      () => memoryFetch({ query: 'q', mode: undefined, limit: 5 }),
      'openhuman.memory_fetch',
      { query: 'q', limit: 5 },
    ],
    [
      'learn',
      () => memoryLearn({ text: 't', kind: 'fact' }),
      'openhuman.memory_learn',
      { text: 't', kind: 'fact' },
    ],
    ['forget', () => memoryForget(['a', 'b']), 'openhuman.memory_forget', { ids: ['a', 'b'] }],
    [
      'items list',
      () => memoryItemsList({ filter: { kinds: ['learning'] }, limit: 20, cursor: 'c' }),
      'openhuman.memory_items_list',
      { filter: { kinds: ['learning'] }, limit: 20, cursor: 'c' },
    ],
    ['conversations get', () => memoryConversationsGet(), 'openhuman.memory_conversations_get', {}],
    [
      'conversations set',
      () => memoryConversationsSet({ batch_turns: 6 }),
      'openhuman.memory_conversations_set',
      { batch_turns: 6 },
    ],
    ['sources list', () => memorySourcesList(), 'openhuman.memory_sources_list', {}],
    [
      'sources add',
      () => memorySourcesAdd({ kind: 'folder', target: '/notes', schedule_mins: 60 }),
      'openhuman.memory_sources_add',
      { kind: 'folder', target: '/notes', schedule_mins: 60 },
    ],
    [
      'sources remove',
      () => memorySourcesRemove('s1', true),
      'openhuman.memory_sources_remove',
      { id: 's1', forget_items: true },
    ],
    ['sources sync all', () => memorySourcesSync(), 'openhuman.memory_sources_sync', {}],
    [
      'sources sync one',
      () => memorySourcesSync('s1'),
      'openhuman.memory_sources_sync',
      { id: 's1' },
    ],
    ['context get', () => memoryContextGet(), 'openhuman.memory_context_get', {}],
    ['context refresh', () => memoryContextRefresh(), 'openhuman.memory_context_refresh', {}],
    [
      'context set',
      () => memoryContextSet({ enabled: false }),
      'openhuman.memory_context_set',
      { enabled: false },
    ],
    ['import scan', () => memoryImportScan(), 'openhuman.memory_import_scan', {}],
    [
      'import start always sends consent',
      () => memoryImportStart(),
      'openhuman.memory_import_start',
      { consent: true },
    ],
    ['import status', () => memoryImportStatus(), 'openhuman.memory_import_status', {}],
  ];

  it.each(cases)('%s', async (_name, invoke, method, params) => {
    await invoke();
    expect(rpc).toHaveBeenCalledWith({ method, params });
  });
});

describe('memoryApi responses', () => {
  it('returns the payload as-is', async () => {
    const state = { engine: 'tinyhumans', has_key: false, status: 'ok', fetch_modes: ['hybrid'] };
    rpc.mockResolvedValue(state);
    await expect(memoryEngineGet()).resolves.toEqual(state);
  });

  it('unwraps the { result, logs } controller envelope', async () => {
    rpc.mockResolvedValue({ result: { forgotten: 2 }, logs: ['x'] });
    await expect(memoryForget(['a'])).resolves.toEqual({ forgotten: 2 });
  });

  it('rethrows RPC failures', async () => {
    rpc.mockRejectedValue(new Error('MEMORY_OFF: no engine'));
    await expect(memoryRecall({ question: 'q' })).rejects.toThrow('MEMORY_OFF');
  });
});

describe('memoryErrorCode', () => {
  it('reads data.code', () => {
    expect(memoryErrorCode({ message: 'x', data: { code: 'UNSUPPORTED' } })).toBe('UNSUPPORTED');
  });

  it('reads data.kind', () => {
    expect(memoryErrorCode({ message: 'x', data: { kind: 'MEMORY_OFF' } })).toBe('MEMORY_OFF');
  });

  it('falls back to a code prefix on the message', () => {
    expect(memoryErrorCode(new Error('UNAUTHORIZED: bad key'))).toBe('UNAUTHORIZED');
  });

  it('returns null for an unrelated error', () => {
    expect(memoryErrorCode(new Error('boom'))).toBeNull();
    expect(memoryErrorCode({ data: { code: 'OTHER' } })).toBeNull();
    expect(memoryErrorCode(null)).toBeNull();
  });
});

describe('helpers', () => {
  it('memoryErrorMessage handles errors, objects and primitives', () => {
    expect(memoryErrorMessage(new Error('a'))).toBe('a');
    expect(memoryErrorMessage({ message: 'b' })).toBe('b');
    expect(memoryErrorMessage('c')).toBe('c');
  });

  it('isMemoryOn needs an engine that is not off', () => {
    expect(isMemoryOn(null)).toBe(false);
    expect(isMemoryOn({ engine: null, has_key: false, status: 'off', fetch_modes: [] })).toBe(
      false
    );
    expect(isMemoryOn({ engine: 'cortexdb', has_key: true, status: 'off', fetch_modes: [] })).toBe(
      false
    );
    expect(
      isMemoryOn({ engine: 'cortexdb', has_key: true, status: 'degraded', fetch_modes: [] })
    ).toBe(true);
  });
});
