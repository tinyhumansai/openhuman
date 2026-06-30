import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { worktreeApi } from './worktreeApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockRpc = vi.mocked(callCoreRpc);

describe('worktreeApi', () => {
  beforeEach(() => {
    mockRpc.mockReset();
  });

  it('list calls worktree_list and returns the view', async () => {
    const view = { worktrees: [], overlaps: [] };
    mockRpc.mockResolvedValueOnce(view);
    const out = await worktreeApi.list();
    expect(mockRpc).toHaveBeenCalledWith({ method: 'openhuman.worktree_list', params: {} });
    expect(out).toBe(view);
  });

  it('status forwards the path', async () => {
    const status = { path: '/r/.claude/worktrees/a', isDirty: false, changedFiles: [] };
    mockRpc.mockResolvedValueOnce(status);
    const out = await worktreeApi.status('/r/.claude/worktrees/a');
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.worktree_status',
      params: { path: '/r/.claude/worktrees/a' },
    });
    expect(out).toBe(status);
  });

  it('status rejects a blank path without calling RPC', async () => {
    await expect(worktreeApi.status('  ')).rejects.toThrow('path is required');
    expect(mockRpc).not.toHaveBeenCalled();
  });

  it('diff unwraps the summary string', async () => {
    mockRpc.mockResolvedValueOnce({ summary: ' src/a.rs | 2 +-' });
    const out = await worktreeApi.diff('/r/.claude/worktrees/a');
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.worktree_diff',
      params: { path: '/r/.claude/worktrees/a' },
    });
    expect(out).toBe(' src/a.rs | 2 +-');
  });

  it('remove defaults force to false and unwraps removed', async () => {
    mockRpc.mockResolvedValueOnce({ removed: true });
    const out = await worktreeApi.remove('/r/.claude/worktrees/a');
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.worktree_remove',
      params: { path: '/r/.claude/worktrees/a', force: false },
    });
    expect(out).toBe(true);
  });

  it('remove forwards force=true', async () => {
    mockRpc.mockResolvedValueOnce({ removed: true });
    await worktreeApi.remove('/r/.claude/worktrees/a', true);
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.worktree_remove',
      params: { path: '/r/.claude/worktrees/a', force: true },
    });
  });
});
