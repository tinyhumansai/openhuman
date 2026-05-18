/**
 * Vitest for the vault tauriCommands surface. Mirrors the pattern used by
 * `subconscious.test.ts` — mocks `callCoreRpc` + `isTauri` so the wrappers
 * are validated against the live RPC contract without spinning up Tauri.
 */
import { afterEach, beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri } from './common';

vi.mock('./common', async () => {
  const actual = await vi.importActual<typeof import('./common')>('./common');
  return { ...actual, isTauri: vi.fn() };
});
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands/vault', () => {
  const mockIsTauri = isTauri as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;
  let openhumanVaultList: typeof import('./vault').openhumanVaultList;
  let openhumanVaultCreate: typeof import('./vault').openhumanVaultCreate;
  let openhumanVaultGet: typeof import('./vault').openhumanVaultGet;
  let openhumanVaultFiles: typeof import('./vault').openhumanVaultFiles;
  let openhumanVaultRemove: typeof import('./vault').openhumanVaultRemove;
  let openhumanVaultSync: typeof import('./vault').openhumanVaultSync;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const actual = await vi.importActual<typeof import('./vault')>('./vault');
    openhumanVaultList = actual.openhumanVaultList;
    openhumanVaultCreate = actual.openhumanVaultCreate;
    openhumanVaultGet = actual.openhumanVaultGet;
    openhumanVaultFiles = actual.openhumanVaultFiles;
    openhumanVaultRemove = actual.openhumanVaultRemove;
    openhumanVaultSync = actual.openhumanVaultSync;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('openhumanVaultList', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanVaultList()).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('dispatches openhuman.vault_list with no params', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: [], logs: [] });
      const resp = await openhumanVaultList();
      expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.vault_list' });
      expect(resp.result).toEqual([]);
    });
  });

  describe('openhumanVaultCreate', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanVaultCreate({ name: 'n', rootPath: '/x' })).rejects.toThrow(
        'Not running in Tauri'
      );
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('forwards optional glob arrays with snake_case keys', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: { id: 'v-1' }, logs: [] });
      await openhumanVaultCreate({
        name: 'notes',
        rootPath: '/Users/me/notes',
        includeGlobs: ['*.md'],
        excludeGlobs: ['drafts'],
      });
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_create',
        params: {
          name: 'notes',
          root_path: '/Users/me/notes',
          include_globs: ['*.md'],
          exclude_globs: ['drafts'],
        },
      });
    });

    test('defaults include/exclude globs to empty arrays when omitted', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: { id: 'v-2' }, logs: [] });
      await openhumanVaultCreate({ name: 'n', rootPath: '/y' });
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_create',
        params: { name: 'n', root_path: '/y', include_globs: [], exclude_globs: [] },
      });
    });
  });

  describe('openhumanVaultGet', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanVaultGet('v-1')).rejects.toThrow('Not running in Tauri');
    });

    test('dispatches openhuman.vault_get with vault_id', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: { id: 'v-1' }, logs: [] });
      await openhumanVaultGet('v-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_get',
        params: { vault_id: 'v-1' },
      });
    });
  });

  describe('openhumanVaultFiles', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanVaultFiles('v-1')).rejects.toThrow('Not running in Tauri');
    });

    test('dispatches openhuman.vault_files with vault_id', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: [], logs: [] });
      await openhumanVaultFiles('v-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_files',
        params: { vault_id: 'v-1' },
      });
    });
  });

  describe('openhumanVaultRemove', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanVaultRemove('v-1', false)).rejects.toThrow('Not running in Tauri');
    });

    test('forwards purge_memory=true', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { vault_id: 'v-1', removed: true, purged: true },
        logs: [],
      });
      await openhumanVaultRemove('v-1', true);
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_remove',
        params: { vault_id: 'v-1', purge_memory: true },
      });
    });

    test('forwards purge_memory=false', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { vault_id: 'v-1', removed: true, purged: false },
        logs: [],
      });
      await openhumanVaultRemove('v-1', false);
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_remove',
        params: { vault_id: 'v-1', purge_memory: false },
      });
    });
  });

  describe('openhumanVaultSync', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanVaultSync('v-1')).rejects.toThrow('Not running in Tauri');
    });

    test('dispatches openhuman.vault_sync with vault_id and returns report', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: {
          vault_id: 'v-1',
          scanned: 3,
          ingested: 2,
          unchanged: 1,
          removed: 0,
          failed: 0,
          skipped_unsupported: 0,
          duration_ms: 12,
          errors: [],
        },
        logs: [],
      });
      const resp = await openhumanVaultSync('v-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_sync',
        params: { vault_id: 'v-1' },
      });
      expect(resp.result.ingested).toBe(2);
    });
  });
});
