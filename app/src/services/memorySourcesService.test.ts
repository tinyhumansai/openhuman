import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from './coreRpcClient';
import {
  addMemorySource,
  listMemorySources,
  removeMemorySource,
  SOURCE_KIND_ICONS,
  SOURCE_KIND_LABEL_KEYS,
  updateMemorySource,
} from './memorySourcesService';

vi.mock('./coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockedCall = vi.mocked(callCoreRpc);

describe('memorySourcesService', () => {
  beforeEach(() => {
    mockedCall.mockReset();
  });

  it('listMemorySources returns sources from envelope-wrapped response', async () => {
    mockedCall.mockResolvedValue({
      result: {
        sources: [{ id: 'src_1', kind: 'folder', label: 'Notes', enabled: true, path: '/tmp' }],
      },
      logs: [],
    } as never);

    const sources = await listMemorySources();

    expect(mockedCall).toHaveBeenCalledWith({ method: 'openhuman.memory_sources_list' });
    expect(sources).toHaveLength(1);
    expect(sources[0].kind).toBe('folder');
  });

  it('listMemorySources handles flat (un-wrapped) response', async () => {
    mockedCall.mockResolvedValue({ sources: [] } as never);
    const sources = await listMemorySources();
    expect(sources).toEqual([]);
  });

  it('addMemorySource sends kind-specific flat fields', async () => {
    mockedCall.mockResolvedValue({
      result: {
        source: { id: 'src_new', kind: 'folder', label: 'Test', enabled: true, path: '/x' },
      },
      logs: [],
    } as never);

    const result = await addMemorySource({
      kind: 'folder',
      label: 'Test',
      enabled: true,
      path: '/x',
    });

    expect(mockedCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_sources_add',
      params: { kind: 'folder', label: 'Test', enabled: true, path: '/x' },
    });
    expect(result.id).toBe('src_new');
  });

  it('updateMemorySource sends id + patch fields', async () => {
    mockedCall.mockResolvedValue({
      result: { source: { id: 'src_1', kind: 'folder', label: 'X', enabled: false } },
      logs: [],
    } as never);

    await updateMemorySource('src_1', { enabled: false, label: 'X' });

    expect(mockedCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_sources_update',
      params: { id: 'src_1', enabled: false, label: 'X' },
    });
  });

  it('removeMemorySource returns boolean', async () => {
    mockedCall.mockResolvedValue({ result: { removed: true }, logs: [] } as never);
    const removed = await removeMemorySource('src_1');
    expect(removed).toBe(true);
  });

  it('exposes labels and icons for every source kind', () => {
    const kinds = [
      'composio',
      'folder',
      'github_repo',
      'twitter_query',
      'rss_feed',
      'web_page',
    ] as const;
    for (const kind of kinds) {
      expect(SOURCE_KIND_LABEL_KEYS[kind]).toBeTruthy();
      expect(SOURCE_KIND_ICONS[kind]).toBeTruthy();
    }
  });
});
