/**
 * IntelligenceWorktreesTab — the isolated-worktree command-center surface (#3376).
 *
 * Reads `openhuman.worktree_list` (via {@link worktreeApi}) and renders every
 * managed worker `git worktree` (those under `<repo>/.claude/worktrees`) with
 * its branch, dirty status, and changed files, plus per-row open / diff /
 * remove actions. A cross-worktree overlap banner warns when two workers
 * changed the same file so the user reconciles before merging.
 *
 * Mirrors {@link IntelligenceAgentWorkTab}'s mount pattern (mountedRef + a 0ms
 * `setTimeout` so the first paint shows the loading state). Removing a worktree
 * refetches the list so the row drops and overlaps recompute.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { worktreeApi, type WorktreeListView } from '../../services/api/worktreeApi';
import { basename } from '../../utils/pathUtils';
import WorktreeActions from '../worktree/WorktreeActions';

const log = debug('intelligence:worktrees');

export default function IntelligenceWorktreesTab() {
  const { t } = useT();
  const [data, setData] = useState<WorktreeListView | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const fetchWorktrees = useCallback(async () => {
    log('fetchWorktrees: entry');
    setError(null);
    try {
      const view = await worktreeApi.list();
      if (mountedRef.current) {
        setData(view);
        log('fetchWorktrees: done count=%d', view.worktrees.length);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('fetchWorktrees: error %s', msg);
      if (mountedRef.current) setError(msg);
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => void fetchWorktrees(), 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [fetchWorktrees]);

  const onRemoved = useCallback(() => {
    void fetchWorktrees();
  }, [fetchWorktrees]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-10 text-content-faint">
        <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
        <span className="text-sm">{t('worktree.panel.loading')}</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-xl border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
        {t('worktree.panel.failedToLoad')}: {error}
      </div>
    );
  }

  if (!data || data.worktrees.length === 0) {
    return (
      <div className="space-y-4">
        <p className="text-xs text-content-faint">{t('worktree.panel.subtitle')}</p>
        <div className="rounded-xl border border-dashed border-line py-10 text-center text-sm text-content-faint">
          {t('worktree.panel.empty')}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <p className="text-xs text-content-faint">{t('worktree.panel.subtitle')}</p>

      {data.overlaps.length > 0 ? (
        <div
          className="space-y-1 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200"
          data-testid="worktree-overlaps">
          <p className="font-medium">{t('worktree.panel.overlapsTitle')}</p>
          <p className="text-amber-700 dark:text-amber-300">{t('worktree.panel.overlapHint')}</p>
          <ul className="mt-1 space-y-0.5">
            {data.overlaps.map(o => (
              <li key={o.file} className="font-mono">
                {o.file}
                <span className="ml-1 font-sans text-amber-600 dark:text-amber-400">
                  ({o.branches.join(', ')})
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <ul className="divide-y divide-line-subtle overflow-hidden rounded-xl border border-line bg-surface dark:divide-neutral-800">
        {data.worktrees.map(wt => (
          <li key={wt.path} className="space-y-2 p-3" data-testid="worktree-row">
            <div className="flex flex-wrap items-center gap-2">
              <span className="truncate text-sm font-medium text-content" title={wt.path}>
                {basename(wt.path)}
              </span>
              {wt.branch ? (
                <span className="rounded-md border border-line px-1.5 py-0.5 font-mono text-[10px] text-content-muted">
                  {wt.branch}
                </span>
              ) : null}
              {wt.isDirty ? (
                <span className="rounded-full bg-amber-100 px-2 py-0.5 text-[10px] font-medium text-amber-700 dark:bg-amber-500/15 dark:text-amber-300">
                  {t('worktree.dirty')}
                </span>
              ) : (
                <span className="rounded-full bg-sage-100 px-2 py-0.5 text-[10px] font-medium text-sage-700 dark:bg-sage-500/15 dark:text-sage-300">
                  {t('worktree.clean')}
                </span>
              )}
              {wt.changedFiles.length > 0 ? (
                <span className="text-[11px] text-content-faint">
                  {wt.changedFiles.length}{' '}
                  {wt.changedFiles.length === 1
                    ? t('worktree.changedFile')
                    : t('worktree.changedFiles')}
                </span>
              ) : null}
            </div>

            {wt.changedFiles.length > 0 ? (
              <ul className="ml-1 space-y-0.5 font-mono text-[11px] text-content-muted">
                {wt.changedFiles.slice(0, 8).map(f => (
                  <li key={f} className="truncate" title={f}>
                    {f}
                  </li>
                ))}
                {wt.changedFiles.length > 8 ? (
                  <li className="text-content-faint">+{wt.changedFiles.length - 8}</li>
                ) : null}
              </ul>
            ) : null}

            <WorktreeActions path={wt.path} isDirty={wt.isDirty} onRemoved={onRemoved} />
          </li>
        ))}
      </ul>
    </div>
  );
}
