/**
 * UserTaskComposer — modal form for creating a user-owned task.
 *
 * Tasks default to the personal board ({@link USER_TASKS_THREAD_ID}) and
 * are *optionally* attachable to an existing conversation thread. On
 * submit it calls `todosApi.add` and hands the resulting board back to the
 * parent via `onCreated` so the Tasks tab can refresh in place.
 */
import debug from 'debug';
import { useState } from 'react';
import { LuX } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import { todosApi, USER_TASKS_THREAD_ID } from '../../services/api/todosApi';
import { useAppSelector } from '../../store/hooks';
import type { TaskBoard, TaskBoardCardStatus } from '../../types/turnState';
import Button from '../ui/Button';

const log = debug('intelligence:task-composer');

// All user-facing task statuses available in the composer.
const STATUS_OPTIONS: { value: TaskBoardCardStatus; labelKey: string }[] = [
  { value: 'todo', labelKey: 'conversations.taskKanban.pending' },
  { value: 'awaiting_approval', labelKey: 'conversations.taskKanban.awaitingApproval' },
  { value: 'ready', labelKey: 'conversations.taskKanban.ready' },
  { value: 'in_progress', labelKey: 'conversations.taskKanban.working' },
  { value: 'blocked', labelKey: 'conversations.taskKanban.blocked' },
  { value: 'done', labelKey: 'conversations.taskKanban.done' },
  { value: 'rejected', labelKey: 'conversations.taskKanban.rejected' },
];

interface UserTaskComposerProps {
  /** Called with the updated board for the thread the task landed on. */
  onCreated: (threadId: string, board: TaskBoard) => void;
  onClose: () => void;
}

export function UserTaskComposer({ onCreated, onClose }: UserTaskComposerProps) {
  const { t } = useT();
  const threads = useAppSelector(state => state.thread.threads ?? []);

  const [title, setTitle] = useState('');
  const [status, setStatus] = useState<TaskBoardCardStatus>('todo');
  const [objective, setObjective] = useState('');
  const [notes, setNotes] = useState('');
  const [attachThreadId, setAttachThreadId] = useState('');
  // When on, the new personal-board card is assigned to the orchestrator so the
  // task dispatcher's poller auto-picks and runs it. Off → a plain manual todo
  // the poller never touches. Only meaningful on the personal board (the poller
  // doesn't poll attached conversation threads), so it's disabled when attaching.
  const [assignToAgent, setAssignToAgent] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Only user-initiated conversations are attachable; background
  // worker/subagent threads (those with a parent) would be confusing
  // targets for a manual task.
  const attachableThreads = threads.filter(thread => !thread.parentThreadId);

  const canSubmit = title.trim().length > 0 && !submitting;

  const handleSubmit = async () => {
    const trimmedTitle = title.trim();
    if (!trimmedTitle || submitting) return;
    const threadId = attachThreadId || USER_TASKS_THREAD_ID;
    // Auto-pick only works on the personal board (the poller doesn't poll
    // attached conversation threads), so ignore the toggle when attaching.
    const assign = assignToAgent && !attachThreadId;
    setSubmitting(true);
    setError(null);
    log('submit threadId=%s status=%s assign=%s', threadId, status, assign);
    try {
      // Assigning to the orchestrator + waiving the per-card approval gate so
      // the dispatcher's poller picks it up and runs it — done atomically in
      // the single `add` call (no create-then-edit race / partial failure).
      const board = await todosApi.add({
        threadId,
        content: trimmedTitle,
        status,
        objective: objective.trim() || null,
        notes: notes.trim() || null,
        ...(assign ? { assignedAgent: 'orchestrator', approvalMode: 'not_required' } : {}),
      });

      onCreated(threadId, board);
      onClose();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('submit failed: %s', msg);
      setError(msg);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 px-4 py-6">
      <section className="max-h-full w-full max-w-lg overflow-y-auto rounded-lg border border-line bg-surface p-4 shadow-xl">
        <div className="mb-3 flex items-start justify-between gap-3">
          <h3 className="text-base font-semibold text-content">
            {t('intelligence.tasks.composer.title')}
          </h3>
          <button
            type="button"
            aria-label={t('common.cancel')}
            onClick={onClose}
            className="flex h-7 w-7 flex-none items-center justify-center rounded-md text-content-muted hover:bg-surface-hover hover:text-content">
            <LuX className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-3 text-sm">
          <label className="block">
            <span className="mb-1 block text-xs font-semibold text-content-muted">
              {t('intelligence.tasks.composer.titleLabel')}
            </span>
            <input
              autoFocus
              value={title}
              onChange={e => setTitle(e.target.value)}
              onKeyDown={e => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) handleSubmit();
              }}
              placeholder={t('intelligence.tasks.composer.titlePlaceholder')}
              className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas"
            />
          </label>

          <div className="grid gap-3 sm:grid-cols-2">
            <label className="block">
              <span className="mb-1 block text-xs font-semibold text-content-muted">
                {t('intelligence.tasks.composer.statusLabel')}
              </span>
              <select
                value={status}
                onChange={e => setStatus(e.target.value as TaskBoardCardStatus)}
                className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas">
                {STATUS_OPTIONS.map(option => (
                  <option key={option.value} value={option.value}>
                    {t(option.labelKey)}
                  </option>
                ))}
              </select>
            </label>

            <label className="block">
              <span className="mb-1 block text-xs font-semibold text-content-muted">
                {t('intelligence.tasks.composer.attachLabel')}
              </span>
              <select
                value={attachThreadId}
                onChange={e => setAttachThreadId(e.target.value)}
                className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas">
                <option value="">{t('intelligence.tasks.composer.attachNone')}</option>
                {attachableThreads.map(thread => (
                  <option key={thread.id} value={thread.id}>
                    {thread.title?.trim() || thread.id}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <label className="block">
            <span className="mb-1 block text-xs font-semibold text-content-muted">
              {t('intelligence.tasks.composer.objectiveLabel')}
            </span>
            <input
              value={objective}
              onChange={e => setObjective(e.target.value)}
              placeholder={t('intelligence.tasks.composer.objectivePlaceholder')}
              className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas"
            />
          </label>

          <label className="block">
            <span className="mb-1 block text-xs font-semibold text-content-muted">
              {t('intelligence.tasks.composer.notesLabel')}
            </span>
            <textarea
              value={notes}
              onChange={e => setNotes(e.target.value)}
              rows={3}
              placeholder={t('intelligence.tasks.composer.notesPlaceholder')}
              className="w-full resize-y rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-content dark:bg-surface-canvas"
            />
          </label>

          <label className="flex items-start gap-2">
            <input
              type="checkbox"
              checked={assignToAgent && !attachThreadId}
              disabled={attachThreadId !== ''}
              onChange={e => setAssignToAgent(e.target.checked)}
              className="mt-0.5 h-4 w-4 flex-none rounded border-line-strong text-ocean-600 focus:ring-ocean-500 disabled:opacity-50 dark:border-neutral-600 dark:bg-surface-canvas"
            />
            <span className="text-xs text-content-secondary">
              <span className="font-semibold text-content-secondary">
                {t('intelligence.tasks.composer.assignAgentLabel')}
              </span>
              <span className="mt-0.5 block text-content-muted">
                {t('intelligence.tasks.composer.assignAgentHint')}
              </span>
            </span>
          </label>

          {error && (
            <p className="rounded-md border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
              {t('intelligence.tasks.composer.createFailed')}: {error}
            </p>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <Button variant="secondary" size="sm" onClick={onClose}>
              {t('common.cancel')}
            </Button>
            <Button variant="primary" size="sm" onClick={handleSubmit} disabled={!canSubmit}>
              {submitting
                ? t('intelligence.tasks.composer.creating')
                : t('intelligence.tasks.composer.create')}
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
}
