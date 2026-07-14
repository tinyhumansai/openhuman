import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { clearRivMemoryCache } from '../rivCache';
import {
  clearManifestCache,
  defaultMascot,
  fetchMascotManifest,
  findMascot,
  loadManifestRiv,
  parseManifest,
} from './manifestService';
import type { MascotManifest, MascotManifestEntry } from './types';

const TINY: MascotManifestEntry = {
  id: 'tiny-mascot',
  name: 'Tiny Mascot',
  description: 'Default OpenHuman mascot.',
  status: 'ready',
  tags: ['default', 'openhuman'],
  stateEngine: {
    idlePoseCycle: ['idle', 'bookreading', 'dancing'],
    states: { idle: 'idle', thinking: 'thinking' },
    visemeCodes: ['sil', 'PP', 'aa', 'oh', 'ou'],
  },
  files: [
    {
      bytes: 189378,
      path: 'mascots/tiny-mascot/tinyMascot.riv',
      role: 'runtime',
      sha256: 'aaaa',
      url: 'https://raw.githubusercontent.com/tinyhumansai/mascots/main/mascots/tiny-mascot/tinyMascot.riv',
    },
    {
      bytes: 622966,
      path: 'mascots/tiny-mascot/tinyMascot.rev',
      role: 'source',
      sha256: 'bbbb',
      url: 'https://raw.githubusercontent.com/tinyhumansai/mascots/main/mascots/tiny-mascot/tinyMascot.rev',
    },
  ],
};

const TOSHI: MascotManifestEntry = {
  id: 'toshi',
  name: 'Toshi',
  description: 'Uploaded Toshi mascot asset.',
  status: 'draft',
  tags: ['draft', 'openhuman'],
  stateEngine: {
    idlePoseCycle: ['idle', 'look_around', 'dancing'],
    states: { idle: 'idle', thinking: 'look_around' },
    visemeCodes: ['sil', 'PP', 'aa'],
    channels: [
      {
        key: 'eyes',
        label: 'Eyes',
        values: ['blink', 'look_left', 'look_right'],
        cycle: { enabled: true, intervalMs: 2600, order: 'random' },
      },
    ],
  },
  files: [
    {
      bytes: 2259491,
      path: 'mascots/toshi/toshi.riv',
      role: 'runtime',
      sha256: 'cccc',
      url: 'https://raw.githubusercontent.com/tinyhumansai/mascots/main/mascots/toshi/toshi.riv',
    },
  ],
};

function manifestDoc(mascots: unknown[] = [TINY, TOSHI]): unknown {
  return {
    schemaVersion: 1,
    generatedAt: '2026-06-29T21:04:06.960Z',
    mascots,
    source: { repository: 'tinyhumansai/mascots', branch: 'main', commit: 'abc' },
  };
}

function mockFetchJson(doc: unknown) {
  const fn = vi
    .fn()
    .mockResolvedValue({ ok: true, json: () => Promise.resolve(doc) } as unknown as Response);
  vi.stubGlobal('fetch', fn);
  return fn;
}

beforeEach(() => {
  clearManifestCache();
  clearRivMemoryCache();
  window.localStorage.clear();
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('parseManifest', () => {
  it('accepts a well-formed document', () => {
    const m = parseManifest(manifestDoc());
    expect(m.mascots.map(x => x.id)).toEqual(['tiny-mascot', 'toshi']);
  });

  it('drops entries missing a runtime .riv', () => {
    const noRuntime = { ...TINY, id: 'broken', files: [TINY.files[1]] };
    const m = parseManifest(manifestDoc([TINY, noRuntime]));
    expect(m.mascots.map(x => x.id)).toEqual(['tiny-mascot']);
  });

  it('drops entries with an incomplete stateEngine', () => {
    const noStates = {
      ...TOSHI,
      id: 'bad',
      stateEngine: { ...TOSHI.stateEngine, states: { idle: 'idle' } },
    };
    const m = parseManifest(manifestDoc([TINY, noStates]));
    expect(m.mascots.map(x => x.id)).toEqual(['tiny-mascot']);
  });

  it('drops entries with malformed stateEngine arrays or channels', () => {
    const badVisemes = {
      ...TOSHI,
      id: 'bad-visemes',
      stateEngine: { ...TOSHI.stateEngine, visemeCodes: ['sil', 123] },
    };
    const badChannels = {
      ...TOSHI,
      id: 'bad-channels',
      stateEngine: { ...TOSHI.stateEngine, channels: [{ key: 'eyes', values: [] }] },
    };
    const m = parseManifest(manifestDoc([TINY, badVisemes, badChannels]));
    expect(m.mascots.map(x => x.id)).toEqual(['tiny-mascot']);
  });

  it('throws when there are no renderable mascots', () => {
    expect(() => parseManifest(manifestDoc([]))).toThrow();
    expect(() => parseManifest({ nope: true })).toThrow();
  });
});

describe('fetchMascotManifest', () => {
  it('fetches once and memoises for the session', async () => {
    const fn = mockFetchJson(manifestDoc());
    const a = await fetchMascotManifest();
    const b = await fetchMascotManifest();
    expect(b).toBe(a);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it('falls back to the localStorage snapshot when the network fails', async () => {
    mockFetchJson(manifestDoc());
    await fetchMascotManifest(); // writes snapshot
    clearManifestCache();

    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValue(new Error('offline')) as unknown as typeof fetch
    );
    const m = await fetchMascotManifest();
    expect(m.mascots.map(x => x.id)).toEqual(['tiny-mascot', 'toshi']);
  });

  it('aborts a stalled fetch so the localStorage snapshot can be used', async () => {
    vi.useFakeTimers();
    mockFetchJson(manifestDoc());
    await fetchMascotManifest(); // writes snapshot
    clearManifestCache();

    vi.stubGlobal(
      'fetch',
      vi.fn((_url: string, init?: RequestInit) => {
        return new Promise((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => reject(new Error('aborted')));
        });
      }) as unknown as typeof fetch
    );

    const pending = fetchMascotManifest();
    await vi.advanceTimersByTimeAsync(5_000);
    const m = await pending;
    expect(m.mascots.map(x => x.id)).toEqual(['tiny-mascot', 'toshi']);
  });

  it('rejects when the network fails and there is no snapshot', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValue(new Error('offline')) as unknown as typeof fetch
    );
    await expect(fetchMascotManifest()).rejects.toThrow('offline');
  });
});

describe('selectors', () => {
  const manifest: MascotManifest = parseManifest(manifestDoc());

  it('findMascot resolves by id', () => {
    expect(findMascot(manifest, 'toshi')?.name).toBe('Toshi');
    expect(findMascot(manifest, 'missing')).toBeUndefined();
    expect(findMascot(manifest, null)).toBeUndefined();
  });

  it('defaultMascot prefers the first ready entry', () => {
    expect(defaultMascot(manifest)?.id).toBe('tiny-mascot');
  });
});

describe('loadManifestRiv', () => {
  it('downloads the runtime file keyed by its sha256', async () => {
    const buf = new Uint8Array([0x52, 0x49, 0x56, 0x45]).buffer;
    const fn = vi
      .fn()
      .mockResolvedValue({
        ok: true,
        arrayBuffer: () => Promise.resolve(buf),
      } as unknown as Response);
    vi.stubGlobal('fetch', fn);

    const out = await loadManifestRiv(TINY);
    expect(new Uint8Array(out)[0]).toBe(0x52);
    expect(fn).toHaveBeenCalledWith(TINY.files[0].url);
  });
});
