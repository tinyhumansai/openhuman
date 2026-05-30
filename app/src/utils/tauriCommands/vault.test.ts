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
  let openhumanVaultWriteMarkdown: typeof import('./vault').openhumanVaultWriteMarkdown;
  let openhumanVaultRemove: typeof import('./vault').openhumanVaultRemove;
  let openhumanVaultSync: typeof import('./vault').openhumanVaultSync;
  let openhumanVaultSyncStatus: typeof import('./vault').openhumanVaultSyncStatus;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const actual = await vi.importActual<typeof import('./vault')>('./vault');
    openhumanVaultList = actual.openhumanVaultList;
    openhumanVaultCreate = actual.openhumanVaultCreate;
    openhumanVaultGet = actual.openhumanVaultGet;
    openhumanVaultFiles = actual.openhumanVaultFiles;
    openhumanVaultWriteMarkdown = actual.openhumanVaultWriteMarkdown;
    openhumanVaultRemove = actual.openhumanVaultRemove;
    openhumanVaultSync = actual.openhumanVaultSync;
    openhumanVaultSyncStatus = actual.openhumanVaultSyncStatus;
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

  describe('openhumanVaultWriteMarkdown', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(
        openhumanVaultWriteMarkdown({
          vaultId: 'v-1',
          relPath: 'wiki/a.md',
          content: '# A',
          approved: true,
        })
      ).rejects.toThrow('Not running in Tauri');
    });

    test('dispatches openhuman.vault_write_markdown with approval state', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { vault_id: 'v-1', rel_path: 'wiki/a.md', bytes_written: 3, created: true },
        logs: [],
      });
      await openhumanVaultWriteMarkdown({
        vaultId: 'v-1',
        relPath: 'wiki/a.md',
        content: '# A',
        overwrite: true,
        approved: true,
      });
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_write_markdown',
        params: {
          vault_id: 'v-1',
          rel_path: 'wiki/a.md',
          content: '# A',
          overwrite: true,
          approved: true,
        },
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

    test('dispatches openhuman.vault_sync with vault_id and returns started status', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { status: 'started', vault_id: 'v-1' },
        logs: [],
      });
      const resp = await openhumanVaultSync('v-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_sync',
        params: { vault_id: 'v-1' },
      });
      expect(resp.result.status).toBe('started');
      expect(resp.result.vault_id).toBe('v-1');
    });
  });

  describe('openhumanVaultSyncStatus', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanVaultSyncStatus('v-1')).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('dispatches openhuman.vault_sync_status with vault_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: {
          vault_id: 'v-1',
          status: 'completed',
          scanned: 4,
          ingested: 4,
          unchanged: 0,
          removed: 0,
          failed: 0,
          skipped_unsupported: 0,
          total: 4,
          started_at_ms: 1000,
          finished_at_ms: 2000,
          duration_ms: 1000,
          errors: [],
        },
        logs: [],
      });
      const resp = await openhumanVaultSyncStatus('v-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.vault_sync_status',
        params: { vault_id: 'v-1' },
      });
      expect(resp.result.status).toBe('completed');
      expect(resp.result.vault_id).toBe('v-1');
    });
  });
});
