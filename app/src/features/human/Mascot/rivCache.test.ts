import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { clearRivMemoryCache, loadRivBuffer } from './rivCache';

function riv(byte = 1): ArrayBuffer {
  return new Uint8Array([0x52, 0x49, 0x56, 0x45, byte]).buffer; // "RIVE" + tag
}

describe('rivCache.loadRivBuffer (version-keyed)', () => {
  beforeEach(() => {
    clearRivMemoryCache();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  function mockFetch(buffer: ArrayBuffer) {
    const fn = vi
      .fn()
      .mockResolvedValue({
        ok: true,
        arrayBuffer: () => Promise.resolve(buffer),
      } as unknown as Response);
    vi.stubGlobal('fetch', fn);
    return fn;
  }

  it('fetches once, then serves the same version from cache', async () => {
    const fetchFn = mockFetch(riv(1));

    const a = await loadRivBuffer('toshi', '1.0.0', 'http://x/mascots/toshi/riv?v=1.0.0');
    const b = await loadRivBuffer('toshi', '1.0.0', 'http://x/mascots/toshi/riv?v=1.0.0');

    expect(new Uint8Array(a)[0]).toBe(0x52);
    expect(b).toBe(a); // identical cached buffer
    expect(fetchFn).toHaveBeenCalledTimes(1); // no second network hit
  });

  it('re-fetches when the version changes', async () => {
    const fetchFn = mockFetch(riv(1));
    await loadRivBuffer('toshi', '1.0.0', 'http://x/riv?v=1.0.0');
    expect(fetchFn).toHaveBeenCalledTimes(1);

    await loadRivBuffer('toshi', '2.0.0', 'http://x/riv?v=2.0.0');
    expect(fetchFn).toHaveBeenCalledTimes(2); // version bump invalidates cache
  });

  it('throws on a non-OK response', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: false, status: 404 } as unknown as Response)
    );
    await expect(loadRivBuffer('missing', '1', 'http://x/riv')).rejects.toThrow(/404/);
  });
});

// ---------------------------------------------------------------------------
// IndexedDB persistence — exercised with a minimal in-memory fake so the
// open/read/write/close code paths run under jsdom (which has no IndexedDB).
// ---------------------------------------------------------------------------

interface FakeReq {
  result: unknown;
  error: unknown;
  onsuccess: ((ev: unknown) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  onupgradeneeded?: ((ev: unknown) => void) | null;
}

function makeFakeIndexedDb() {
  const backing = new Map<string, Map<string, unknown>>(); // store -> (key -> value)

  function makeStore(name: string) {
    const map = backing.get(name) ?? new Map();
    backing.set(name, map);
    return {
      get(key: string) {
        const req: FakeReq = { result: undefined, error: null, onsuccess: null, onerror: null };
        setTimeout(() => {
          req.result = map.get(key);
          req.onsuccess?.({ target: req });
        }, 0);
        return req;
      },
      put(value: { id: string }) {
        map.set(value.id, value);
        return { onsuccess: null, onerror: null };
      },
    };
  }

  function makeDb() {
    return {
      objectStoreNames: { contains: (n: string) => backing.has(n) },
      createObjectStore: (n: string) => {
        backing.set(n, backing.get(n) ?? new Map());
      },
      transaction() {
        const tx: {
          oncomplete: (() => void) | null;
          onerror: null;
          onabort: null;
          objectStore: (n: string) => unknown;
        } = {
          oncomplete: null,
          onerror: null,
          onabort: null,
          objectStore: (n: string) => makeStore(n),
        };
        // Resolve writes after the current microtask so put() has run.
        setTimeout(() => tx.oncomplete?.(), 0);
        return tx;
      },
      close() {},
    };
  }

  return {
    open() {
      const req: FakeReq = {
        result: undefined,
        error: null,
        onsuccess: null,
        onerror: null,
        onupgradeneeded: null,
      };
      setTimeout(() => {
        req.result = makeDb();
        req.onupgradeneeded?.({ target: req });
        req.onsuccess?.({ target: req });
      }, 0);
      return req;
    },
  };
}

describe('rivCache IndexedDB persistence', () => {
  beforeEach(() => {
    clearRivMemoryCache();
    vi.stubGlobal('indexedDB', makeFakeIndexedDb());
  });
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('persists to IndexedDB and serves a later mount from it (no refetch)', async () => {
    const fetchFn = vi
      .fn()
      .mockResolvedValue({
        ok: true,
        arrayBuffer: () => Promise.resolve(riv(7)),
      } as unknown as Response);
    vi.stubGlobal('fetch', fetchFn);

    // First load: miss → fetch → write to IDB.
    await loadRivBuffer('toshi', '1.0.0', 'http://x/riv?v=1.0.0');
    expect(fetchFn).toHaveBeenCalledTimes(1);

    // Drop the in-memory layer so the next read must come from IndexedDB.
    clearRivMemoryCache();
    const again = await loadRivBuffer('toshi', '1.0.0', 'http://x/riv?v=1.0.0');
    expect(new Uint8Array(again)[0]).toBe(0x52);
    expect(fetchFn).toHaveBeenCalledTimes(1); // served from IDB, no second fetch
  });

  it('ignores a stale IndexedDB entry when the version changed', async () => {
    const fetchFn = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        arrayBuffer: () => Promise.resolve(riv(1)),
      } as unknown as Response)
      .mockResolvedValueOnce({
        ok: true,
        arrayBuffer: () => Promise.resolve(riv(2)),
      } as unknown as Response);
    vi.stubGlobal('fetch', fetchFn);

    await loadRivBuffer('toshi', '1.0.0', 'http://x/riv?v=1.0.0');
    clearRivMemoryCache();
    await loadRivBuffer('toshi', '2.0.0', 'http://x/riv?v=2.0.0');
    expect(fetchFn).toHaveBeenCalledTimes(2);
  });
});
