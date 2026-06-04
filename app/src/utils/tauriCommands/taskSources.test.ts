/**
 * Vitest for the task-sources tauriCommands surface.
 *
 * Covers each `openhuman.task_sources_*` RPC wrapper plus the
 * `isTauri()` guard. Mirrors the mocking pattern in
 * `subconscious.test.ts` — validates the wrappers against the
 * `callCoreRpc` contract without a real Tauri runtime.
 */
import { afterEach, beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri } from './common';
import {
  openhumanTaskSourcesAdd,
  openhumanTaskSourcesFetch,
  openhumanTaskSourcesGet,
  openhumanTaskSourcesList,
  openhumanTaskSourcesListTasks,
  openhumanTaskSourcesPreviewFilter,
  openhumanTaskSourcesRemove,
  openhumanTaskSourcesStatus,
  openhumanTaskSourcesUpdate,
} from './taskSources';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('./common', () => ({ isTauri: vi.fn() }));

describe('tauriCommands/taskSources', () => {
  const mockIsTauri = isTauri as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;

  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    mockCallCoreRpc.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test('list forwards the list method with no params', async () => {
    mockCallCoreRpc.mockResolvedValue([]);
    await openhumanTaskSourcesList();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.task_sources_list' });
  });

  test('get forwards id', async () => {
    await openhumanTaskSourcesGet('s-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.task_sources_get',
      params: { id: 's-1' },
    });
  });

  test('add forwards the add params verbatim', async () => {
    const params = {
      provider: 'github' as const,
      filter: { provider: 'github' as const, repo: 'o/r', assignee_is_me: true },
      name: 'My issues',
    };
    await openhumanTaskSourcesAdd(params);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.task_sources_add', params });
  });

  test('update forwards id + patch', async () => {
    await openhumanTaskSourcesUpdate('s-1', { enabled: false });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.task_sources_update',
      params: { id: 's-1', patch: { enabled: false } },
    });
  });

  test('remove forwards id', async () => {
    await openhumanTaskSourcesRemove('s-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.task_sources_remove',
      params: { id: 's-1' },
    });
  });

  test('fetch forwards id', async () => {
    await openhumanTaskSourcesFetch('s-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.task_sources_fetch',
      params: { id: 's-1' },
    });
  });

  test('listTasks forwards id + default limit', async () => {
    mockCallCoreRpc.mockResolvedValue([]);
    await openhumanTaskSourcesListTasks('s-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.task_sources_list_tasks',
      params: { id: 's-1', limit: 50 },
    });
  });

  test('listTasks forwards an explicit limit', async () => {
    mockCallCoreRpc.mockResolvedValue([]);
    await openhumanTaskSourcesListTasks('s-1', 10);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.task_sources_list_tasks',
      params: { id: 's-1', limit: 10 },
    });
  });

  test('previewFilter forwards provider/filter/connection/max', async () => {
    mockCallCoreRpc.mockResolvedValue([]);
    await openhumanTaskSourcesPreviewFilter(
      'notion',
      { provider: 'notion', database_id: 'db-1', assigned_to_me: true },
      'conn-1',
      5
    );
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.task_sources_preview_filter',
      params: {
        provider: 'notion',
        filter: { provider: 'notion', database_id: 'db-1', assigned_to_me: true },
        connection_id: 'conn-1',
        max: 5,
      },
    });
  });

  test('status forwards the status method', async () => {
    await openhumanTaskSourcesStatus();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.task_sources_status' });
  });

  test('every wrapper throws and skips RPC when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(openhumanTaskSourcesList()).rejects.toThrow('Not running in Tauri');
    await expect(openhumanTaskSourcesGet('x')).rejects.toThrow('Not running in Tauri');
    await expect(
      openhumanTaskSourcesAdd({ provider: 'github', filter: { provider: 'github' } })
    ).rejects.toThrow('Not running in Tauri');
    await expect(openhumanTaskSourcesUpdate('x', {})).rejects.toThrow('Not running in Tauri');
    await expect(openhumanTaskSourcesRemove('x')).rejects.toThrow('Not running in Tauri');
    await expect(openhumanTaskSourcesFetch('x')).rejects.toThrow('Not running in Tauri');
    await expect(openhumanTaskSourcesListTasks('x')).rejects.toThrow('Not running in Tauri');
    await expect(
      openhumanTaskSourcesPreviewFilter('github', { provider: 'github' })
    ).rejects.toThrow('Not running in Tauri');
    await expect(openhumanTaskSourcesStatus()).rejects.toThrow('Not running in Tauri');
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });
});
