import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { getCachedMascotDetail, loadMascotRivBuffer } from '../../../../services/mascotService';
import { BackendRiveMascot } from './BackendRiveMascot';
import type { MascotDetail, RiveMascotDetail } from './types';

// Capture the props useRive receives so we can assert which source (default
// src vs. custom buffer) the mascot is rendered from.
const h = vi.hoisted(() => ({ useRiveParams: [] as Record<string, unknown>[] }));

vi.mock('@rive-app/react-webgl2', () => ({
  Fit: { Contain: 'contain' },
  Layout: class {
    constructor(opts: unknown) {
      Object.assign(this, opts as object);
    }
  },
  useRive: (params: Record<string, unknown>) => {
    h.useRiveParams.push(params);
    return { rive: {}, RiveComponent: () => null };
  },
  useViewModel: () => ({}),
  useViewModelInstance: () => ({}),
  useViewModelInstanceEnum: () => ({ setValue: () => {}, value: null, values: [] }),
  useViewModelInstanceColor: () => ({ setValue: () => {} }),
}));

vi.mock('../../../../services/mascotService', () => ({
  getCachedMascotDetail: vi.fn(),
  loadMascotRivBuffer: vi.fn(),
}));

const riveDetail: RiveMascotDetail = {
  id: 'toshi',
  name: 'Toshi',
  version: '1.0.0',
  description: '',
  format: 'rive',
  rivFileUrl: '/mascots/toshi/riv?v=1.0.0',
  defaultState: 'idle',
  stateToPose: {},
  viewModelInputs: [],
};

function lastSource(): Record<string, unknown> {
  return h.useRiveParams.at(-1) ?? {};
}

describe('BackendRiveMascot', () => {
  beforeEach(() => {
    h.useRiveParams = [];
    vi.clearAllMocks();
  });

  it('renders the version-cached buffer once the mascot resolves', async () => {
    const buffer = new ArrayBuffer(8);
    vi.mocked(getCachedMascotDetail).mockResolvedValue(riveDetail);
    vi.mocked(loadMascotRivBuffer).mockResolvedValue(buffer);

    render(<BackendRiveMascot mascotId="toshi" face="idle" />);

    // Starts on the bundled default while loading...
    expect(lastSource().src).toBe('/tiny_mascot.riv');

    // ...then swaps to the custom buffer.
    await waitFor(() => expect(lastSource().buffer).toBe(buffer));
    expect(loadMascotRivBuffer).toHaveBeenCalledWith(riveDetail);
  });

  it('falls back to the default mascot when the load fails', async () => {
    vi.mocked(getCachedMascotDetail).mockRejectedValue(new Error('network'));

    render(<BackendRiveMascot mascotId="toshi" />);

    await waitFor(() => expect(getCachedMascotDetail).toHaveBeenCalled());
    // Never rendered a buffer — stayed on the bundled default.
    expect(h.useRiveParams.every(p => p.buffer === undefined)).toBe(true);
    expect(lastSource().src).toBe('/tiny_mascot.riv');
  });

  it('falls back to the default when the selected mascot is an SVG (non-rive)', async () => {
    const svgDetail = { id: 'old', format: 'svg', states: [] } as unknown as MascotDetail;
    vi.mocked(getCachedMascotDetail).mockResolvedValue(svgDetail);

    render(<BackendRiveMascot mascotId="old" />);

    await waitFor(() => expect(getCachedMascotDetail).toHaveBeenCalled());
    expect(loadMascotRivBuffer).not.toHaveBeenCalled();
    expect(lastSource().src).toBe('/tiny_mascot.riv');
  });

  it('renders a canvas container', () => {
    vi.mocked(getCachedMascotDetail).mockResolvedValue(riveDetail);
    vi.mocked(loadMascotRivBuffer).mockResolvedValue(new ArrayBuffer(8));
    const { container } = render(<BackendRiveMascot mascotId="toshi" size={160} />);
    expect(container.querySelector('[data-face]')).not.toBeNull();
    expect(screen).toBeDefined();
  });
});
