/**
 * Unit tests for ManifestRiveMascot — the manifest-driven Rive renderer.
 *
 * Mocks the WebGL Rive runtime (records every enum/color setValue by path)
 * and the manifest .riv loader so we can assert the component:
 *   - falls back to the bundled default while the buffer loads,
 *   - drives pose/viseme constrained to the mascot's stateEngine, and
 *   - auto-cycles a manifest channel (eyes) when idlePoseRotation is on.
 */
import { act, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { MascotManifestEntry } from './manifest/types';
import { ManifestRiveMascot } from './ManifestRiveMascot';

const h = vi.hoisted(() => ({
  useRiveParams: null as Record<string, unknown> | null,
  enumCalls: {} as Record<string, unknown[]>,
  colorCalls: {} as Record<string, unknown[]>,
}));

vi.mock('@rive-app/react-webgl2', () => ({
  Fit: { Contain: 'contain' },
  Layout: class {
    constructor(opts: unknown) {
      Object.assign(this, opts as object);
    }
  },
  useRive: (params: Record<string, unknown>) => {
    h.useRiveParams = params;
    return { rive: {}, RiveComponent: () => null };
  },
  useViewModel: () => ({}),
  useViewModelInstance: () => ({}),
  useViewModelInstanceEnum: (path: string) => ({
    setValue: (v: string) => (h.enumCalls[path] ??= []).push(v),
    value: null,
    values: [],
  }),
  useViewModelInstanceColor: (path: string) => ({
    setValue: (v: number) => (h.colorCalls[path] ??= []).push(v),
  }),
}));

const loadManifestRiv = vi.hoisted(() => vi.fn());
vi.mock('./manifest/manifestService', () => ({ loadManifestRiv }));

const TOSHI: MascotManifestEntry = {
  id: 'toshi',
  name: 'Toshi',
  description: '',
  status: 'ready',
  tags: [],
  stateEngine: {
    idlePoseCycle: ['idle', 'look_around', 'pointing'],
    states: { idle: 'idle', thinking: 'look_around' },
    visemeCodes: ['sil', 'PP', 'aa'],
    channels: [
      {
        key: 'eyes',
        label: 'Eyes',
        values: ['blink', 'look_left', 'look_right'],
        cycle: { enabled: true, intervalMs: 2600, order: 'sequential' },
      },
    ],
  },
  files: [
    { path: 'm/toshi.riv', bytes: 1, role: 'runtime', sha256: 'cccc', url: 'https://x/toshi.riv' },
  ],
};

const RIVER: MascotManifestEntry = {
  ...TOSHI,
  id: 'river-guide',
  name: 'River Guide',
  stateEngine: { ...TOSHI.stateEngine, states: { idle: 'idle', thinking: 'thinking' } },
  files: [
    { path: 'm/river.riv', bytes: 1, role: 'runtime', sha256: 'dddd', url: 'https://x/river.riv' },
  ],
};

function enumLast(path: string): string | undefined {
  return (h.enumCalls[path] ?? []).at(-1) as string | undefined;
}

beforeEach(() => {
  h.useRiveParams = null;
  h.enumCalls = {};
  h.colorCalls = {};
  loadManifestRiv.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('ManifestRiveMascot', () => {
  it('renders the bundled default while the buffer loads', () => {
    loadManifestRiv.mockReturnValue(new Promise(() => {})); // never resolves
    render(<ManifestRiveMascot entry={TOSHI} face="idle" />);
    // Default RiveMascot loads from the bundled src, not a buffer.
    expect(h.useRiveParams?.src).toBe('/tiny_mascot.riv');
  });

  it('renders from the loaded buffer and drives the mascot vocabulary', async () => {
    loadManifestRiv.mockResolvedValue(new Uint8Array([1, 2, 3]).buffer);
    render(<ManifestRiveMascot entry={TOSHI} face="thinking" visemeCode="aa" />);

    await waitFor(() => expect(h.useRiveParams?.buffer).toBeInstanceOf(ArrayBuffer));
    // 'thinking' face → Toshi's look_around (it has no 'thinking' pose).
    expect((h.enumCalls['pose'] ?? []).at(-1)).toBe('look_around');
    expect(enumLast('mouthVisemeCode')).toBe('aa');
  });

  it('clears the previous buffer when the entry changes without a remount', async () => {
    let resolveRiver!: (buffer: ArrayBuffer) => void;
    loadManifestRiv
      .mockResolvedValueOnce(new Uint8Array([1, 2, 3]).buffer)
      .mockReturnValueOnce(new Promise<ArrayBuffer>(res => (resolveRiver = res)));

    const { rerender } = render(<ManifestRiveMascot entry={TOSHI} face="idle" />);
    await waitFor(() => expect(h.useRiveParams?.buffer).toBeInstanceOf(ArrayBuffer));

    rerender(<ManifestRiveMascot entry={RIVER} face="idle" />);
    await waitFor(() => expect(h.useRiveParams?.src).toBe('/tiny_mascot.riv'));

    await act(async () => {
      resolveRiver(new Uint8Array([4, 5, 6]).buffer);
    });
    await waitFor(() => expect(h.useRiveParams?.buffer).toBeInstanceOf(ArrayBuffer));
  });

  it('rests the mouth for a viseme outside the mascot vocabulary', async () => {
    loadManifestRiv.mockResolvedValue(new Uint8Array([1]).buffer);
    render(<ManifestRiveMascot entry={TOSHI} face="speaking" visemeCode="O" />);
    await waitFor(() => expect(h.useRiveParams?.buffer).toBeInstanceOf(ArrayBuffer));
    // Toshi has no 'oh' viseme → rest on sil.
    await waitFor(() => expect(enumLast('mouthVisemeCode')).toBe('sil'));
  });

  it('auto-cycles a cyclable channel when idle rotation is on', async () => {
    vi.useFakeTimers();
    loadManifestRiv.mockResolvedValue(new Uint8Array([1]).buffer);
    render(<ManifestRiveMascot entry={TOSHI} face="idle" idlePoseRotation />);

    // Let the async buffer load resolve.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(h.useRiveParams?.buffer).toBeInstanceOf(ArrayBuffer);

    const before = (h.enumCalls['eyes'] ?? []).length;
    act(() => {
      vi.advanceTimersByTime(2_600); // one cycle interval
    });
    const eyesWrites = h.enumCalls['eyes'] ?? [];
    expect(eyesWrites.length).toBeGreaterThan(before);
    // sequential order starts at values[0] then advances to values[1].
    expect(eyesWrites.at(-1)).toBe('look_left');
  });
});
