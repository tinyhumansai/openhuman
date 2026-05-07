/**
 * Vitest for the subconscious tauriCommands surface (#623).
 *
 * Covers the three RPC wrappers — `listReflections`, `actOnReflection`,
 * `dismissReflection` — plus their `isTauri()` guard. Mirrors the
 * mocking pattern used by `config.test.ts` and `core.test.ts` so the
 * wrappers are validated against the live `callCoreRpc` contract
 * without spinning up a real Tauri runtime.
 */
import { isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands/subconscious', () => {
  const mockIsTauri = isTauri as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;
  let listReflections: typeof import('./subconscious').listReflections;
  let actOnReflection: typeof import('./subconscious').actOnReflection;
  let dismissReflection: typeof import('./subconscious').dismissReflection;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const actual = await vi.importActual<typeof import('./subconscious')>('./subconscious');
    listReflections = actual.listReflections;
    actOnReflection = actual.actOnReflection;
    dismissReflection = actual.dismissReflection;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('listReflections', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(listReflections()).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('forwards default limit and omits since_ts when absent', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: [], logs: [] });
      await listReflections();
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.subconscious_reflections_list',
        params: { limit: 50 },
      });
    });

    test('passes through explicit limit + since_ts when supplied', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: [], logs: [] });
      await listReflections(20, 1700000000);
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.subconscious_reflections_list',
        params: { limit: 20, since_ts: 1700000000 },
      });
    });
  });

  describe('actOnReflection', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(actOnReflection('r-1')).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('dispatches openhuman.subconscious_reflections_act with reflection_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { reflection_id: 'r-1', thread_id: 'thr-9' },
        logs: [],
      });
      const resp = await actOnReflection('r-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.subconscious_reflections_act',
        params: { reflection_id: 'r-1' },
      });
      expect(resp.result.thread_id).toBe('thr-9');
    });
  });

  describe('dismissReflection', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(dismissReflection('r-1')).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('dispatches openhuman.subconscious_reflections_dismiss with reflection_id', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: { dismissed: 'r-1' }, logs: [] });
      await dismissReflection('r-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.subconscious_reflections_dismiss',
        params: { reflection_id: 'r-1' },
      });
    });
  });
});
