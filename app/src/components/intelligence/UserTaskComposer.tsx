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

const log = debug('intelligence:task-composer');

// Tasks use three states only: Pending / Working / Done.
const STATUS_OPTIONS: { value: TaskBoardCardStatus; labelKey: string }[] = [
  { value: 'todo', labelKey: 'conversations.taskKanban.pending' },
  { value: 'in_progress', labelKey: 'conversations.taskKanban.working' },
  { value: 'done', labelKey: 'conversations.taskKanban.done' },
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
    setSubmitting(true);
    setError(null);
    log('submit threadId=%s status=%s', threadId, status);
    try {
      const board = await todosApi.add({
        threadId,
        content: trimmedTitle,
        status,
        objective: objective.trim() || null,
        notes: notes.trim() || null,
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
      <section className="max-h-full w-full max-w-lg overflow-y-auto rounded-lg border border-stone-200 bg-white p-4 shadow-xl dark:border-neutral-800 dark:bg-neutral-900">
        <div className="mb-3 flex items-start justify-between gap-3">
          <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-50">
            {t('intelligence.tasks.composer.title')}
          </h3>
          <button
            type="button"
            aria-label={t('common.cancel')}
            onClick={onClose}
            className="flex h-7 w-7 flex-none items-center justify-center rounded-md text-stone-500 hover:bg-stone-100 hover:text-stone-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100">
            <LuX className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-3 text-sm">
          <label className="block">
            <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
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
              className="w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50"
            />
          </label>

          <div className="grid gap-3 sm:grid-cols-2">
            <label className="block">
              <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
                {t('intelligence.tasks.composer.statusLabel')}
              </span>
              <select
                value={status}
                onChange={e => setStatus(e.target.value as TaskBoardCardStatus)}
                className="w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50">
                {STATUS_OPTIONS.map(option => (
                  <option key={option.value} value={option.value}>
                    {t(option.labelKey)}
                  </option>
                ))}
              </select>
            </label>

            <label className="block">
              <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
                {t('intelligence.tasks.composer.attachLabel')}
              </span>
              <select
                value={attachThreadId}
                onChange={e => setAttachThreadId(e.target.value)}
                className="w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50">
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
            <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
              {t('intelligence.tasks.composer.objectiveLabel')}
            </span>
            <input
              value={objective}
              onChange={e => setObjective(e.target.value)}
              placeholder={t('intelligence.tasks.composer.objectivePlaceholder')}
              className="w-full rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50"
            />
          </label>

          <label className="block">
            <span className="mb-1 block text-xs font-semibold text-stone-500 dark:text-neutral-400">
              {t('intelligence.tasks.composer.notesLabel')}
            </span>
            <textarea
              value={notes}
              onChange={e => setNotes(e.target.value)}
              rows={3}
              placeholder={t('intelligence.tasks.composer.notesPlaceholder')}
              className="w-full resize-y rounded-md border border-stone-200 bg-white px-2 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-50"
            />
          </label>

          {error && (
            <p className="rounded-md border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
              {t('intelligence.tasks.composer.createFailed')}: {error}
            </p>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md border border-stone-200 px-3 py-1.5 text-xs font-medium text-stone-600 hover:bg-stone-50 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800">
              {t('common.cancel')}
            </button>
            <button
              type="button"
              onClick={handleSubmit}
              disabled={!canSubmit}
              className="rounded-md bg-ocean-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-ocean-700 disabled:opacity-50">
              {submitting
                ? t('intelligence.tasks.composer.creating')
                : t('intelligence.tasks.composer.create')}
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}
