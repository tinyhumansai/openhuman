import { describe, expect, it, vi } from 'vitest';

// The global setup mocks isTauri to return false by default.
// We need to selectively override it for these tests.

// Re-mock tauriCommands so we can test the actual implementations
// rather than the blanket mock from setup.ts.
vi.mock('../tauriCommands', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../tauriCommands');
  return actual;
});

// Mock @tauri-apps/api/core — isTauri controls the guard in each function
const mockIsTauri = vi.fn(() => false);
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: () => mockIsTauri(),
}));

// Mock callCoreRpc — the underlying transport for all memory commands
const mockCallCoreRpc = vi.fn();
vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

import { memoryDocIngest, memoryGraphQuery } from '../tauriCommands';

describe('memoryGraphQuery', () => {
  it('throws when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(memoryGraphQuery()).rejects.toThrow('Not running in Tauri');
  });

  it('calls core RPC with memory.graph.query method and optional params', async () => {
    mockIsTauri.mockReturnValue(true);
    const mockRelations = [
      {
        namespace: 'team',
        subject: 'Alice',
        predicate: 'OWNS',
        object: 'Atlas',
        attrs: {},
        updatedAt: 1700000000,
        evidenceCount: 2,
        orderIndex: null,
        documentIds: ['doc-1'],
        chunkIds: ['doc-1#chunk-1'],
      },
    ];
    mockCallCoreRpc.mockResolvedValue(mockRelations);

    const result = await memoryGraphQuery('team', 'Alice', 'OWNS');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'memory.graph.query',
      params: { namespace: 'team', subject: 'Alice', predicate: 'OWNS' },
    });
    expect(result).toEqual(mockRelations);
  });

  it('passes undefined params when called with no arguments', async () => {
    mockIsTauri.mockReturnValue(true);
    mockCallCoreRpc.mockResolvedValue([]);

    await memoryGraphQuery();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'memory.graph.query',
      params: { namespace: undefined, subject: undefined, predicate: undefined },
    });
  });
});

describe('memoryDocIngest', () => {
  it('throws when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(
      memoryDocIngest({ namespace: 'ns', key: 'k', title: 't', content: 'c' })
    ).rejects.toThrow('Not running in Tauri');
  });

  it('calls core RPC with memory.doc.ingest and forwards all params', async () => {
    mockIsTauri.mockReturnValue(true);
    const ingestResult = { document_id: 'doc-123', entity_count: 3, relation_count: 2, chunk_count: 5 };
    mockCallCoreRpc.mockResolvedValue(ingestResult);

    const params = {
      namespace: 'research',
      key: 'paper-1',
      title: 'Memory Paper',
      content: 'Some content about memory systems',
      source_type: 'paper',
      priority: 'high',
      tags: ['memory', 'ai'],
      metadata: { author: 'Alice' },
      category: 'research',
    };

    const result = await memoryDocIngest(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'memory.doc.ingest',
      params,
    });
    expect(result).toEqual(ingestResult);
  });

  it('sends only required fields when optional fields are omitted', async () => {
    mockIsTauri.mockReturnValue(true);
    mockCallCoreRpc.mockResolvedValue({});

    const params = { namespace: 'ns', key: 'k', title: 't', content: 'c' };
    await memoryDocIngest(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'memory.doc.ingest',
      params,
    });
  });
});
