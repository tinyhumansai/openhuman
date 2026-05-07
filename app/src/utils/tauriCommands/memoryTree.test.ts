/**
 * Unit tests for memory_tree RPC wrappers. Mirror the pattern used by
 * `memory.test.ts` — mock the underlying `callCoreRpc` and assert that
 * each helper dispatches the right method name + params and unwraps
 * `RpcOutcome`'s `{ result, logs }` envelope correctly.
 */
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import {
  memoryTreeChunkScore,
  memoryTreeDeleteChunk,
  memoryTreeEntityIndexFor,
  memoryTreeFlushNow,
  memoryTreeGetLlm,
  memoryTreeGraphExport,
  memoryTreeListChunks,
  memoryTreeListSources,
  memoryTreeRecall,
  memoryTreeResetTree,
  memoryTreeSearch,
  memoryTreeSetLlm,
  memoryTreeTopEntities,
  memoryTreeWipeAll,
} from './memoryTree';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCallCoreRpc = callCoreRpc as Mock;

beforeEach(() => {
  vi.clearAllMocks();
});

describe('memoryTreeListChunks', () => {
  test('dispatches openhuman.memory_tree_list_chunks with the filter as params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: { chunks: [], total: 0 },
      logs: ['memory_tree::read: list_chunks n=0 total=0'],
    });

    const out = await memoryTreeListChunks({ limit: 50 });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_list_chunks',
      params: { limit: 50 },
    });
    expect(out).toEqual({ chunks: [], total: 0 });
  });

  test('handles bare-value responses (no logs envelope)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ chunks: [{ id: 'c1' }], total: 1 });
    const out = await memoryTreeListChunks({});
    expect(out.total).toBe(1);
    expect(out.chunks[0]?.id).toBe('c1');
  });
});

describe('memoryTreeListSources', () => {
  test('omits user_email_hint when not provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [], logs: ['stub'] });

    await memoryTreeListSources();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_list_sources',
      params: {},
    });
  });

  test('forwards user_email_hint when provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [], logs: ['stub'] });

    await memoryTreeListSources('alice@example.com');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_list_sources',
      params: { user_email_hint: 'alice@example.com' },
    });
  });

  test('returns the unwrapped Source array', async () => {
    const sources = [
      {
        source_id: 'gmail:x|y',
        display_name: 'X',
        source_kind: 'email',
        chunk_count: 2,
        most_recent_ms: 1,
      },
    ];
    mockCallCoreRpc.mockResolvedValueOnce({ result: sources, logs: ['stub'] });
    const out = await memoryTreeListSources();
    expect(out).toEqual(sources);
  });
});

describe('memoryTreeSearch', () => {
  test('dispatches with query + k', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [], logs: ['stub'] });

    await memoryTreeSearch('phoenix', 25);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_search',
      params: { query: 'phoenix', k: 25 },
    });
  });
});

describe('memoryTreeRecall', () => {
  test('dispatches with query + k and unwraps the recall envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: { chunks: [{ id: 'c1' }], scores: [0.9] },
      logs: ['stub'],
    });

    const out = await memoryTreeRecall('design sync', 10);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_recall',
      params: { query: 'design sync', k: 10 },
    });
    expect(out.chunks).toHaveLength(1);
    expect(out.scores[0]).toBe(0.9);
  });
});

describe('memoryTreeEntityIndexFor', () => {
  test('dispatches with chunk_id', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [], logs: ['stub'] });

    await memoryTreeEntityIndexFor('chunk-abc');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_entity_index_for',
      params: { chunk_id: 'chunk-abc' },
    });
  });
});

describe('memoryTreeTopEntities', () => {
  test('omits kind when not provided and defaults limit to 50', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [], logs: ['stub'] });

    await memoryTreeTopEntities();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_top_entities',
      params: { limit: 50 },
    });
  });

  test('forwards kind + custom limit when provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [], logs: ['stub'] });

    await memoryTreeTopEntities('person', 12);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_top_entities',
      params: { limit: 12, kind: 'person' },
    });
  });
});

describe('memoryTreeChunkScore', () => {
  test('returns null when the core reports no score row', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: null, logs: ['stub'] });

    const out = await memoryTreeChunkScore('chunk-missing');

    expect(out).toBeNull();
  });

  test('unwraps the breakdown when present', async () => {
    const breakdown = {
      signals: [{ name: 'token_count', weight: 1, value: 0.5 }],
      total: 0.5,
      threshold: 0.85,
      kept: false,
      llm_consulted: false,
    };
    mockCallCoreRpc.mockResolvedValueOnce({ result: breakdown, logs: ['stub'] });

    const out = await memoryTreeChunkScore('chunk-real');

    expect(out).toEqual(breakdown);
  });
});

describe('memoryTreeDeleteChunk', () => {
  test('dispatches with chunk_id and surfaces the full DeleteChunkResponse', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: { deleted: true, score_rows_removed: 1, entity_index_rows_removed: 3 },
      logs: ['stub'],
    });

    const out = await memoryTreeDeleteChunk('chunk-xyz');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_delete_chunk',
      params: { chunk_id: 'chunk-xyz' },
    });
    expect(out).toEqual({ deleted: true, score_rows_removed: 1, entity_index_rows_removed: 3 });
  });
});

describe('memoryTreeGetLlm / memoryTreeSetLlm', () => {
  test('get_llm dispatches without params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: { current: 'cloud' }, logs: ['stub'] });

    const out = await memoryTreeGetLlm();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.memory_tree_get_llm' });
    expect(out.current).toBe('cloud');
  });

  test('set_llm dispatches with backend param and returns the effective value', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: { current: 'local' }, logs: ['stub'] });

    const out = await memoryTreeSetLlm('local');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_set_llm',
      params: { backend: 'local' },
    });
    expect(out.current).toBe('local');
  });

  test('set_llm forwards optional per-role model fields verbatim as snake_case', async () => {
    // The wrapper takes either a bare backend string (legacy) or the full
    // request object. When the caller passes a request, the snake_case
    // field names must reach the wire untouched — no camelCase
    // translation lives in this layer.
    mockCallCoreRpc.mockResolvedValueOnce({ result: { current: 'local' }, logs: ['stub'] });

    const out = await memoryTreeSetLlm({
      backend: 'local',
      extract_model: 'qwen2.5:0.5b',
      summariser_model: 'gemma3:1b-it-qat',
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_set_llm',
      params: {
        backend: 'local',
        extract_model: 'qwen2.5:0.5b',
        summariser_model: 'gemma3:1b-it-qat',
      },
    });
    expect(out.current).toBe('local');
  });

  test('set_llm with cloud_model only flips backend + cloud model', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: { current: 'cloud' }, logs: ['stub'] });

    await memoryTreeSetLlm({ backend: 'cloud', cloud_model: 'summarizer-v2' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_set_llm',
      params: { backend: 'cloud', cloud_model: 'summarizer-v2' },
    });
  });
});

describe('memoryTreeFlushNow', () => {
  test('dispatches flush_now and returns the unwrapped envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: { enqueued: true, stale_buffers: 4 },
      logs: ['stub'],
    });

    const out = await memoryTreeFlushNow();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.memory_tree_flush_now' });
    expect(out).toEqual({ enqueued: true, stale_buffers: 4 });
  });

  test('passes through bare-shape responses (no envelope) unchanged', async () => {
    // Defensive path: if a future Rust handler stops emitting logs the
    // bare value flows through `unwrapResult` unchanged.
    mockCallCoreRpc.mockResolvedValueOnce({ enqueued: false, stale_buffers: 0 });

    const out = await memoryTreeFlushNow();

    expect(out).toEqual({ enqueued: false, stale_buffers: 0 });
  });
});

describe('memoryTreeWipeAll', () => {
  test('dispatches wipe_all and returns the unwrapped envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: { rows_deleted: 12, dirs_removed: ['raw', 'wiki'], sync_state_cleared: 1 },
      logs: ['stub'],
    });

    const out = await memoryTreeWipeAll();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.memory_tree_wipe_all' });
    expect(out.rows_deleted).toBe(12);
    expect(out.dirs_removed).toEqual(['raw', 'wiki']);
    expect(out.sync_state_cleared).toBe(1);
  });
});

describe('memoryTreeResetTree', () => {
  test('dispatches reset_tree and returns the unwrapped envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: { tree_rows_deleted: 8, chunks_requeued: 5, jobs_enqueued: 5 },
      logs: ['stub'],
    });

    const out = await memoryTreeResetTree();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.memory_tree_reset_tree' });
    expect(out).toEqual({ tree_rows_deleted: 8, chunks_requeued: 5, jobs_enqueued: 5 });
  });
});

describe('memoryTreeGraphExport', () => {
  test('defaults to mode=tree and returns the unwrapped envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: { nodes: [], edges: [], content_root_abs: '/tmp/workspace/memory_tree/content' },
      logs: ['stub'],
    });

    const out = await memoryTreeGraphExport();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_graph_export',
      params: { mode: 'tree' },
    });
    expect(out.nodes).toEqual([]);
    expect(out.edges).toEqual([]);
    expect(out.content_root_abs).toBe('/tmp/workspace/memory_tree/content');
  });

  test('forwards explicit mode=contacts to the wire params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: {
        nodes: [{ kind: 'chunk', id: 'c1', label: 'one' }],
        edges: [{ from: 'c1', to: 'p1' }],
        content_root_abs: '/tmp/x',
      },
      logs: ['stub'],
    });

    const out = await memoryTreeGraphExport('contacts');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.memory_tree_graph_export',
      params: { mode: 'contacts' },
    });
    expect(out.nodes).toHaveLength(1);
    expect(out.edges).toHaveLength(1);
  });
});
