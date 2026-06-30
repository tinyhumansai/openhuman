import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { RiveMascotDetail } from '../features/human/Mascot/backend/types';
import { loadRivBuffer } from '../features/human/Mascot/rivCache';
import { getBackendUrl } from './backendUrl';
import { loadMascotRivBuffer } from './mascotService';

vi.mock('../features/human/Mascot/rivCache', () => ({ loadRivBuffer: vi.fn() }));
vi.mock('./backendUrl', () => ({ getBackendUrl: vi.fn() }));
vi.mock('./apiClient', () => ({ apiClient: { get: vi.fn() } }));

const rive: RiveMascotDetail = {
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

describe('loadMascotRivBuffer', () => {
  beforeEach(() => {
    vi.mocked(getBackendUrl).mockResolvedValue('https://api.example.test');
    vi.mocked(loadRivBuffer).mockResolvedValue(new ArrayBuffer(8));
  });

  it('composes the absolute, version-stamped URL and delegates to the cache', async () => {
    const buf = await loadMascotRivBuffer(rive);
    expect(buf).toBeInstanceOf(ArrayBuffer);
    expect(loadRivBuffer).toHaveBeenCalledWith(
      'toshi',
      '1.0.0',
      'https://api.example.test/mascots/toshi/riv?v=1.0.0'
    );
  });
});
