import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { getToolkitCatalog, invalidateToolkitCatalogCache } from './catalogCache';

const mockListToolkits = vi.fn();

vi.mock('./composioApi', () => ({ listToolkits: () => mockListToolkits() }));

describe('catalogCache', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
    window.localStorage.clear();
    invalidateToolkitCatalogCache();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('fetches once then serves the cache within the TTL', async () => {
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'], catalog: [] });

    const first = await getToolkitCatalog();
    const second = await getToolkitCatalog();

    expect(first.toolkits).toEqual(['gmail']);
    expect(second.toolkits).toEqual(['gmail']);
    expect(mockListToolkits).toHaveBeenCalledTimes(1);
  });

  it('dedupes concurrent callers into a single fetch', async () => {
    let resolve: (v: unknown) => void = () => {};
    mockListToolkits.mockReturnValue(
      new Promise(r => {
        resolve = r;
      })
    );

    const a = getToolkitCatalog();
    const b = getToolkitCatalog();
    resolve({ toolkits: ['github'], catalog: [] });

    await Promise.all([a, b]);
    expect(mockListToolkits).toHaveBeenCalledTimes(1);
  });

  it('re-fetches after the cache is invalidated', async () => {
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'], catalog: [] });

    await getToolkitCatalog();
    invalidateToolkitCatalogCache();
    await getToolkitCatalog();

    expect(mockListToolkits).toHaveBeenCalledTimes(2);
  });

  it('serves a stale cache when a refresh fails', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00Z'));
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'], catalog: [] });
    await getToolkitCatalog();

    // Advance past the 24h TTL and make the refresh fail.
    vi.setSystemTime(new Date('2026-01-03T00:00:00Z'));
    mockListToolkits.mockRejectedValue(new Error('backend down'));

    const result = await getToolkitCatalog();
    expect(result.toolkits).toEqual(['gmail']); // stale, but better than erroring
  });

  it('propagates the error on a cold-cache failure', async () => {
    mockListToolkits.mockRejectedValue(new Error('backend down'));
    await expect(getToolkitCatalog()).rejects.toThrow('backend down');
  });

  it('does not cache a response whose fetch was invalidated mid-flight', async () => {
    // First fetch is in flight when the Composio client identity switches
    // (mode toggle / BYO key → composio:config-changed → invalidate).
    let resolveFirst: (v: unknown) => void = () => {};
    mockListToolkits.mockReturnValueOnce(
      new Promise(r => {
        resolveFirst = r;
      })
    );

    const inflightCall = getToolkitCatalog();
    invalidateToolkitCatalogCache(); // mid-flight: previous tenant's response is now stale
    resolveFirst({ toolkits: ['old_tenant'], catalog: [] });
    await inflightCall; // the original caller still receives its response

    // The stale response must NOT have been written as a fresh 24h cache, so
    // the next read issues a fresh RPC and serves the new tenant's catalog.
    mockListToolkits.mockResolvedValueOnce({ toolkits: ['new_tenant'], catalog: [] });
    const next = await getToolkitCatalog();

    expect(next.toolkits).toEqual(['new_tenant']);
    expect(mockListToolkits).toHaveBeenCalledTimes(2);
  });
});
