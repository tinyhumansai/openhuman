/**
 * WorktreeActions — open / diff / remove affordances for one isolated worker
 * `git worktree` (#3376). Shared between the inline subagent row (compact) in
 * the conversation timeline and the Worktrees command-center panel (full).
 *
 * Safety contract (acceptance criterion of #3376): a **clean** worktree removes
 * in one click; a **dirty** worktree requires an explicit user decision — the
 * Remove button expands into a confirm row offering "Discard & remove" (force)
 * or "Preserve" (cancel, keep the worktree + its `worker/*` branch). The core
 * itself also refuses a dirty removal without `force`, so this is defense in
 * depth, not the only guard.
 */
import debug from 'debug';
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { worktreeApi } from '../../services/api/worktreeApi';
import { revealPath } from '../../utils/openUrl';

const log = debug('worktree:actions');

export interface WorktreeActionsProps {
  /** Absolute worktree checkout path. */
  path: string;
  /** Whether the worktree has uncommitted changes. */
  isDirty?: boolean;
  /** Compact styling for the inline subagent row (vs the full panel row). */
  compact?: boolean;
  /** Called after a successful remove so the parent can refetch / drop the row. */
  onRemoved?: (path: string) => void;
}

const btnBase =
  'rounded-md border px-2 py-0.5 font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50';
const btnNeutral = 'border-line text-content-secondary hover:bg-surface-hover';
const btnDanger =
  'border-coral-200 text-coral-600 hover:bg-coral-50 dark:border-coral-500/40 dark:text-coral-300 dark:hover:bg-coral-500/10';

export default function WorktreeActions({
  path,
  isDirty,
  compact,
  onRemoved,
}: WorktreeActionsProps) {
  const { t } = useT();
  const [diffOpen, setDiffOpen] = useState(false);
  const [diffText, setDiffText] = useState<string | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sizeText = compact ? 'text-[10px]' : 'text-xs';

  const handleOpen = useCallback(() => {
    log('open path=%s', path);
    void revealPath(path).catch(err => {
      log('open failed %O', err);
      setError(err instanceof Error ? err.message : String(err));
    });
  }, [path]);

  const handleDiff = useCallback(async () => {
    if (diffOpen) {
      setDiffOpen(false);
      return;
    }
    setDiffOpen(true);
    setError(null);
    // Fetch once; cache for subsequent toggles.
    if (diffText !== null) return;
    setDiffLoading(true);
    try {
      const summary = await worktreeApi.diff(path);
      setDiffText(summary);
      log('diff loaded path=%s chars=%d', path, summary.length);
    } catch (err) {
      log('diff failed %O', err);
      setError(t('worktree.diffFailed'));
      setDiffOpen(false);
    } finally {
      setDiffLoading(false);
    }
  }, [diffOpen, diffText, path, t]);

  const doRemove = useCallback(
    async (force: boolean) => {
      setRemoving(true);
      setError(null);
      try {
        await worktreeApi.remove(path, force);
        log('removed path=%s force=%s', path, force);
        // The parent (e.g. the Worktrees panel) drops the row on success and
        // this component unmounts. But inline in the chat timeline
        // (`ToolTimelineBlock`) there is no `onRemoved`, so nothing unmounts
        // us — reset local state here or the row stays stuck on "Removing…"
        // with the button disabled forever even though the checkout is gone.
        if (onRemoved) {
          onRemoved(path);
        } else {
          setRemoving(false);
          setConfirmRemove(false);
        }
      } catch (err) {
        log('remove failed %O', err);
        setError(t('worktree.removeFailed'));
        setRemoving(false);
        setConfirmRemove(false);
      }
    },
    [path, onRemoved, t]
  );

  const handleRemoveClick = useCallback(() => {
    // Dirty worktrees must not be removed without an explicit choice.
    if (isDirty) {
      setConfirmRemove(true);
      return;
    }
    void doRemove(false);
  }, [isDirty, doRemove]);

  return (
    <div className={`space-y-1.5 ${sizeText}`} data-testid="worktree-actions">
      {!confirmRemove ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <button
            type="button"
            onClick={handleOpen}
            className={`${btnBase} ${btnNeutral}`}
            data-testid="worktree-open">
            {t('worktree.action.open')}
          </button>
          <button
            type="button"
            onClick={() => void handleDiff()}
            className={`${btnBase} ${btnNeutral}`}
            data-testid="worktree-diff">
            {diffOpen ? t('worktree.action.hideDiff') : t('worktree.action.diff')}
          </button>
          <button
            type="button"
            onClick={handleRemoveClick}
            disabled={removing}
            className={`${btnBase} ${btnDanger}`}
            data-testid="worktree-remove">
            {removing ? t('worktree.removing') : t('worktree.action.remove')}
          </button>
        </div>
      ) : (
        <div
          className="space-y-1.5 rounded-md border border-coral-200 bg-coral-50 p-2 dark:border-coral-500/40 dark:bg-coral-500/10"
          data-testid="worktree-remove-confirm">
          <p className="text-coral-700 dark:text-coral-300">{t('worktree.dirtyConfirm')}</p>
          <div className="flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              onClick={() => void doRemove(true)}
              disabled={removing}
              className={`${btnBase} ${btnDanger}`}
              data-testid="worktree-remove-confirm-yes">
              {removing ? t('worktree.removing') : t('worktree.action.removeAnyway')}
            </button>
            <button
              type="button"
              onClick={() => setConfirmRemove(false)}
              disabled={removing}
              className={`${btnBase} ${btnNeutral}`}
              data-testid="worktree-preserve">
              {t('worktree.action.preserve')}
            </button>
          </div>
        </div>
      )}

      {diffOpen ? (
        <pre
          className="max-h-48 overflow-auto rounded-md border border-line bg-surface-muted p-2 font-mono text-[10px] leading-snug text-content-secondary"
          data-testid="worktree-diff-output">
          {diffLoading
            ? t('worktree.diffLoading')
            : diffText && diffText.trim().length > 0
              ? diffText
              : t('worktree.diffEmpty')}
        </pre>
      ) : null}

      {error ? (
        <p className="text-coral-600 dark:text-coral-300" data-testid="worktree-error">
          {error}
        </p>
      ) : null}
    </div>
  );
}
