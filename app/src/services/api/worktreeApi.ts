/**
 * Frontend client for the git-worktree manager (#3376) — the isolated worker
 * worktrees created by `spawn_parallel_agents` when a worker opts into
 * `isolation = "worktree"`.
 *
 * Wraps four read/cleanup RPCs under the `worktree` namespace:
 * - `openhuman.worktree_list`   — managed worktrees + cross-worker overlaps.
 * - `openhuman.worktree_status` — branch / dirty / changed-files for one path.
 * - `openhuman.worktree_diff`   — human-readable `--stat` diff for one path.
 * - `openhuman.worktree_remove` — remove a worktree (refuses dirty unless force).
 *
 * The wire payloads are already camelCase (the Rust `WorktreeStatus` serializes
 * with `#[serde(rename_all = "camelCase")]`), so this client only types the
 * shape — no snake/camel transform.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('worktreeApi');

/** Snapshot of a single worktree. Mirrors the Rust `WorktreeStatus`. */
export interface WorktreeStatus {
  /** Absolute path to the worktree checkout. */
  path: string;
  /** Checked-out branch (or `(detached HEAD)`), when resolvable. */
  branch?: string | null;
  /** Whether the worktree has uncommitted changes. */
  isDirty: boolean;
  /** Paths (relative to the worktree root) that differ from HEAD. */
  changedFiles: string[];
}

/** A file changed by more than one worker worktree (pre-merge conflict hint). */
export interface WorktreeOverlap {
  /** Relative path touched by multiple worktrees. */
  file: string;
  /** Branches (or path fallbacks) of the worktrees that touched it. */
  branches: string[];
}

/** Response from `openhuman.worktree_list`. */
export interface WorktreeListView {
  worktrees: WorktreeStatus[];
  overlaps: WorktreeOverlap[];
}

export const worktreeApi = {
  /** List managed worker worktrees plus cross-worktree file overlaps. */
  list: async (): Promise<WorktreeListView> => {
    log('list');
    const view = await callCoreRpc<WorktreeListView>({
      method: 'openhuman.worktree_list',
      params: {},
    });
    log('list received count=%d overlaps=%d', view.worktrees.length, view.overlaps.length);
    return view;
  },

  /** Fetch the branch / dirty / changed-files snapshot for one worktree. */
  status: async (path: string): Promise<WorktreeStatus> => {
    if (!path.trim()) throw new Error('worktreeApi.status: path is required');
    log('status path=%s', path);
    return callCoreRpc<WorktreeStatus>({ method: 'openhuman.worktree_status', params: { path } });
  },

  /** Fetch a human-readable `git diff HEAD --stat` (plus untracked files). */
  diff: async (path: string): Promise<string> => {
    if (!path.trim()) throw new Error('worktreeApi.diff: path is required');
    log('diff path=%s', path);
    const res = await callCoreRpc<{ summary: string }>({
      method: 'openhuman.worktree_diff',
      params: { path },
    });
    return res.summary;
  },

  /**
   * Remove a worktree checkout. The core refuses a dirty worktree unless
   * `force` is `true`, so a clean worktree removes silently while a dirty one
   * rejects (the caller surfaces a confirm prompt and retries with force).
   */
  remove: async (path: string, force = false): Promise<boolean> => {
    if (!path.trim()) throw new Error('worktreeApi.remove: path is required');
    log('remove path=%s force=%s', path, force);
    const res = await callCoreRpc<{ removed: boolean }>({
      method: 'openhuman.worktree_remove',
      params: { path, force },
    });
    return res.removed;
  },
};
