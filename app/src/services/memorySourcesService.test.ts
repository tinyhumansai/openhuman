import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from './coreRpcClient';
import {
  addMemorySource,
  applyAllIn,
  listMemorySources,
  type MemorySourceEntry,
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

  // ---------------------------------------------------------------------------
  // applyAllIn
  // ---------------------------------------------------------------------------

  it('applyAllIn calls the correct RPC method', async () => {
    mockedCall.mockResolvedValue({
      result: {
        sources: [{ id: 'src_1', kind: 'github_repo', label: 'All Repos', enabled: true }],
        sync_triggered: 1,
      },
      logs: [],
    } as never);

    const result = await applyAllIn();

    expect(mockedCall).toHaveBeenCalledWith({ method: 'openhuman.memory_sources_apply_all_in' });
    expect(result.sync_triggered).toBe(1);
    expect(result.sources).toHaveLength(1);
    expect(result.sources[0].id).toBe('src_1');
  });

  it('applyAllIn handles envelope-wrapped response', async () => {
    mockedCall.mockResolvedValue({ result: { sources: [], sync_triggered: 0 }, logs: [] } as never);

    const result = await applyAllIn();
    expect(result.sources).toEqual([]);
    expect(result.sync_triggered).toBe(0);
  });

  it('applyAllIn handles flat (un-wrapped) response', async () => {
    mockedCall.mockResolvedValue({
      sources: [{ id: 'src_2', kind: 'folder', label: 'Docs', enabled: true }],
      sync_triggered: 1,
    } as never);

    const result = await applyAllIn();
    expect(result.sources).toHaveLength(1);
  });

  // ---------------------------------------------------------------------------
  // MemorySourceEntry interface includes all limit fields
  // ---------------------------------------------------------------------------

  it('MemorySourceEntry interface accepts all limit fields at compile-time', () => {
    // This test is a type-level assertion: if the fields are missing from the
    // interface the assignment below would fail TypeScript compilation.
    const entry: MemorySourceEntry = {
      id: 'src_1',
      kind: 'github_repo',
      label: 'Test',
      enabled: true,
      max_commits: 100,
      max_issues: 200,
      max_prs: 50,
      max_items: 500,
      since_days: 30,
      sync_depth_days: 90,
      max_tokens_per_sync: 100_000,
      max_cost_per_sync_usd: 1.5,
    };
    expect(entry.max_commits).toBe(100);
    expect(entry.max_issues).toBe(200);
    expect(entry.max_prs).toBe(50);
    expect(entry.since_days).toBe(30);
    expect(entry.sync_depth_days).toBe(90);
    expect(entry.max_tokens_per_sync).toBe(100_000);
    expect(entry.max_cost_per_sync_usd).toBe(1.5);
  });
});
