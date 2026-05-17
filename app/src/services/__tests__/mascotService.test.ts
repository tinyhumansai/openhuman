import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  clearMascotDetailCache,
  fetchMascotDetail,
  fetchMascotList,
  getCachedMascotDetail,
} from '../mascotService';

const getMock = vi.fn();

vi.mock('../apiClient', () => ({ apiClient: { get: (...args: unknown[]) => getMock(...args) } }));

const summary = {
  id: 'yellow',
  name: 'Yellow',
  version: '1.0.0',
  description: '',
  states: [{ id: 'idle', label: 'Idle', description: '' }],
  hasVisemes: true,
};

const detail = {
  id: 'yellow',
  name: 'Yellow',
  version: '1.0.0',
  description: '',
  viewBox: '0 0 1 1',
  defaultState: 'idle',
  variables: [],
  states: [{ id: 'idle', label: 'Idle', description: '', svg: '<svg/>' }],
  visemes: [],
};

describe('mascotService', () => {
  beforeEach(() => {
    getMock.mockReset();
    clearMascotDetailCache();
  });

  it('fetchMascotList hits /mascots and unwraps the response', async () => {
    getMock.mockResolvedValueOnce({ success: true, data: { mascots: [summary] } });
    const list = await fetchMascotList();
    expect(list).toEqual([summary]);
    expect(getMock).toHaveBeenCalledWith('/mascots', { requireAuth: false });
  });

  it('fetchMascotDetail encodes the id and unwraps the manifest', async () => {
    getMock.mockResolvedValueOnce({ success: true, data: { mascot: detail } });
    const d = await fetchMascotDetail('yellow bob');
    expect(d).toEqual(detail);
    expect(getMock).toHaveBeenCalledWith('/mascots/yellow%20bob', { requireAuth: false });
  });

  it('rejects an empty id without hitting the network', async () => {
    await expect(fetchMascotDetail('   ')).rejects.toThrow(/empty/i);
    expect(getMock).not.toHaveBeenCalled();
  });

  it('getCachedMascotDetail memoizes per id', async () => {
    getMock.mockResolvedValue({ success: true, data: { mascot: detail } });
    const a = await getCachedMascotDetail('yellow');
    const b = await getCachedMascotDetail('yellow');
    expect(a).toBe(b);
    expect(getMock).toHaveBeenCalledTimes(1);
  });

  it('clearMascotDetailCache forces a refetch', async () => {
    getMock.mockResolvedValue({ success: true, data: { mascot: detail } });
    await getCachedMascotDetail('yellow');
    clearMascotDetailCache();
    await getCachedMascotDetail('yellow');
    expect(getMock).toHaveBeenCalledTimes(2);
  });
});
