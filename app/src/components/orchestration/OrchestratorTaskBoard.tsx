/**
 * OrchestratorTaskBoard — the orchestrator's single, app-wide Kanban board.
 *
 * One global board (keyed by {@link ORCHESTRATOR_TASKS_THREAD_ID}) that the
 * orchestrator owns and the user can edit. Deliberately NOT per-thread: thread-
 * scoped to-dos / goals live under Tiny Agents, so this surface shows exactly
 * one board — no aggregation of agent/thread boards.
 *
 * Fully editable via the `todos_*` RPC (each mutation returns the fresh board,
 * which we set directly). Renders the shared {@link TaskKanbanBoard}.
 */
import debugFactory from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  type ColumnDef,
  TaskKanbanBoard,
} from '../../features/conversations/components/TaskKanbanBoard';
import { useT } from '../../lib/i18n/I18nContext';
import { ORCHESTRATOR_TASKS_THREAD_ID, todosApi } from '../../services/api/todosApi';
import type { TaskBoard, TaskBoardCard, TaskBoardCardStatus } from '../../types/turnState';
import Button from '../ui/Button';

const debug = debugFactory('orchestration:task-board');

const EMPTY_BOARD: TaskBoard = { threadId: ORCHESTRATOR_TASKS_THREAD_ID, cards: [], updatedAt: '' };

/** The orchestrator board is a simple 4-column board. */
const ORCHESTRATOR_COLUMNS: ColumnDef[] = [
  { status: 'todo', labelKey: 'orchPage.tasks.colPending' },
  { status: 'in_progress', labelKey: 'orchPage.tasks.colActive' },
  { status: 'blocked', labelKey: 'orchPage.tasks.colBlocked' },
  { status: 'done', labelKey: 'orchPage.tasks.colCompleted' },
];

export default function OrchestratorTaskBoard() {
  const { t } = useT();
  const [board, setBoard] = useState<TaskBoard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const [adding, setAdding] = useState(false);
  const [mutatingCardId, setMutatingCardId] = useState<string | null>(null);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const load = useCallback(async () => {
    try {
      const next = await todosApi.list(ORCHESTRATOR_TASKS_THREAD_ID);
      if (!mountedRef.current) return;
      setBoard(next);
      setError(null);
    } catch (err) {
      debug('load failed: %o', err);
      if (!mountedRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
      setBoard(prev => prev ?? EMPTY_BOARD);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Every todos_* mutation returns the fresh board — apply it directly.
  const runMutation = useCallback(async (cardId: string | null, op: () => Promise<TaskBoard>) => {
    setMutatingCardId(cardId);
    setError(null);
    try {
      const next = await op();
      if (mountedRef.current) setBoard(next);
    } catch (err) {
      debug('mutation failed card=%s: %o', cardId, err);
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setMutatingCardId(null);
    }
  }, []);

  const handleAdd = useCallback(async () => {
    const content = draft.trim();
    if (!content || adding) return;
    setAdding(true);
    setError(null);
    try {
      const next = await todosApi.add({ threadId: ORCHESTRATOR_TASKS_THREAD_ID, content });
      if (!mountedRef.current) return;
      setBoard(next);
      setDraft('');
    } catch (err) {
      debug('add failed: %o', err);
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setAdding(false);
    }
  }, [draft, adding]);

  const handleMove = useCallback(
    (card: TaskBoardCard, status: TaskBoardCardStatus) => {
      void runMutation(card.id, () =>
        todosApi.updateStatus(ORCHESTRATOR_TASKS_THREAD_ID, card.id, status)
      );
    },
    [runMutation]
  );

  const handleUpdate = useCallback(
    (card: TaskBoardCard, nextCard: TaskBoardCard) => {
      void runMutation(card.id, () =>
        todosApi.edit({
          threadId: ORCHESTRATOR_TASKS_THREAD_ID,
          id: card.id,
          content: nextCard.title,
          status: nextCard.status,
          objective: nextCard.objective ?? null,
          notes: nextCard.notes ?? null,
          blocker: nextCard.blocker ?? null,
          assignedAgent: nextCard.assignedAgent ?? null,
          approvalMode: nextCard.approvalMode ?? null,
          plan: nextCard.plan ?? [],
          allowedTools: nextCard.allowedTools ?? [],
          acceptanceCriteria: nextCard.acceptanceCriteria ?? [],
          evidence: nextCard.evidence ?? [],
        })
      );
    },
    [runMutation]
  );

  const handleDelete = useCallback(
    (card: TaskBoardCard) => {
      void runMutation(card.id, () => todosApi.remove(ORCHESTRATOR_TASKS_THREAD_ID, card.id));
    },
    [runMutation]
  );

  return (
    <div className="space-y-4" data-testid="orch-task-board">
      <div>
        <h2 className="text-xl font-bold text-content">{t('orchPage.tasks.nav')}</h2>
        <p className="mt-1 text-sm text-content-muted">{t('orchPage.tasks.subtitle')}</p>
      </div>

      {error ? (
        <div className="flex items-center justify-between gap-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
          <span className="min-w-0 truncate">{error}</span>
          <Button variant="secondary" size="sm" onClick={() => void load()}>
            {t('common.retry')}
          </Button>
        </div>
      ) : null}

      <form
        className="flex gap-2"
        onSubmit={event => {
          event.preventDefault();
          void handleAdd();
        }}>
        <input
          value={draft}
          onChange={event => setDraft(event.target.value)}
          placeholder={t('subconscious.addTaskPlaceholder')}
          data-testid="orch-task-add-input"
          className="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-2 text-sm text-content outline-none transition focus:border-primary-500 focus:ring-2 focus:ring-primary-500/20"
        />
        <Button
          type="submit"
          variant="primary"
          size="sm"
          data-testid="orch-task-add-submit"
          disabled={!draft.trim() || adding}>
          {t('common.add')}
        </Button>
      </form>

      <TaskKanbanBoard
        board={board ?? EMPTY_BOARD}
        columns={ORCHESTRATOR_COLUMNS}
        onMove={handleMove}
        onUpdateCard={handleUpdate}
        onDeleteCard={handleDelete}
        mutatingCardId={mutatingCardId}
        hideHeader
      />
    </div>
  );
}
