/**
 * IntelligenceTasksTab — shows all task boards across the workspace.
 *
 * Surfaces three sources, in priority order:
 *  1. The user's personal board ({@link USER_TASKS_THREAD_ID}), pinned to
 *     the top. This is the only board editable here — users create, move,
 *     edit, and delete their own cards via the `todos_*` RPC.
 *  2. Live agent boards from `chatRuntime.taskBoardByThread` (updated in
 *     real-time while a conversation runs via socket events).
 *  3. Persisted agent boards fetched once on mount from
 *     `threadApi.listTurnStates` (each turn state may carry a `taskBoard`).
 *
 * Agent boards (2 + 3) stay read-only here — those cards are managed from
 * the Conversations page where the agent write path lives.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { LuPlus } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import { TaskKanbanBoard } from '../../pages/conversations/components/TaskKanbanBoard';
import { threadApi } from '../../services/api/threadApi';
import { todosApi, USER_TASKS_THREAD_ID } from '../../services/api/todosApi';
import { useAppSelector } from '../../store/hooks';
import type { TaskBoard, TaskBoardCard, TaskBoardCardStatus } from '../../types/turnState';
import { UserTaskComposer } from './UserTaskComposer';

const log = debug('intelligence:tasks');

interface ThreadTaskBoard {
  threadId: string;
  title: string;
  board: TaskBoard;
  live: boolean;
}

function shortId(threadId: string): string {
  return threadId.length > 8 ? `…${threadId.slice(-8)}` : threadId;
}

export default function IntelligenceTasksTab() {
  const { t } = useT();
  const liveBoards = useAppSelector(state => state.chatRuntime.taskBoardByThread);
  const threads = useAppSelector(state => state.thread.threads ?? []);

  const [persistedBoards, setPersistedBoards] = useState<Record<string, TaskBoard>>({});
  const [personalBoard, setPersonalBoard] = useState<TaskBoard | null>(null);
  const [composerOpen, setComposerOpen] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const fetchPersistedBoards = useCallback(async () => {
    log('fetchPersistedBoards: entry');
    setError(null);
    try {
      const turnStates = await threadApi.listTurnStates();
      log('fetchPersistedBoards: received %d turn states', turnStates.length);
      const boards: Record<string, TaskBoard> = {};
      for (const ts of turnStates) {
        if (ts.taskBoard && ts.taskBoard.cards.length > 0) {
          boards[ts.threadId] = ts.taskBoard;
        }
      }
      if (mountedRef.current) {
        setPersistedBoards(boards);
        log('fetchPersistedBoards: done boards=%d', Object.keys(boards).length);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('fetchPersistedBoards: error %s', msg);
      if (mountedRef.current) setError(msg);
    }
  }, []);

  const fetchPersonalBoard = useCallback(async () => {
    log('fetchPersonalBoard: entry');
    try {
      const board = await todosApi.list(USER_TASKS_THREAD_ID);
      if (mountedRef.current) {
        setPersonalBoard(board);
        log('fetchPersonalBoard: cards=%d', board.cards.length);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('fetchPersonalBoard: error %s', msg);
      // A missing personal board is expected on first run — fall back to
      // an empty board so the create affordance still has a home.
      if (mountedRef.current) {
        setPersonalBoard({ threadId: USER_TASKS_THREAD_ID, cards: [], updatedAt: '' });
      }
    }
  }, []);

  const loadAll = useCallback(async () => {
    // `loading` defaults to true; flip it off once both fetches settle.
    await Promise.allSettled([fetchPersistedBoards(), fetchPersonalBoard()]);
    if (mountedRef.current) setLoading(false);
  }, [fetchPersistedBoards, fetchPersonalBoard]);

  useEffect(() => {
    mountedRef.current = true;
    void loadAll();
    return () => {
      mountedRef.current = false;
    };
  }, [loadAll]);

  // A task created from the composer lands either on the personal board or
  // on a chosen conversation thread. `add` returns the updated board, so we
  // merge it directly — re-fetching listTurnStates would return a stale
  // turn-state snapshot that doesn't reflect the just-added card.
  const handleCreated = useCallback((threadId: string, board: TaskBoard) => {
    log('handleCreated threadId=%s cards=%d', threadId, board.cards.length);
    if (threadId === USER_TASKS_THREAD_ID) {
      setPersonalBoard(board);
    } else {
      setPersistedBoards(prev => ({ ...prev, [threadId]: board }));
    }
  }, []);

  // ── personal-board mutations (optimistic, with rollback) ─────────────

  const mutatePersonal = useCallback(
    async (optimistic: TaskBoard, call: () => Promise<TaskBoard>, previous: TaskBoard) => {
      setActionError(null);
      setPersonalBoard(optimistic);
      try {
        const saved = await call();
        if (mountedRef.current) setPersonalBoard(saved);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('personal board mutation failed: %s', msg);
        if (mountedRef.current) {
          setPersonalBoard(previous);
          setActionError(t('conversations.taskKanban.updateFailed'));
        }
      }
    },
    [t]
  );

  const handleMovePersonal = useCallback(
    (card: TaskBoardCard, status: TaskBoardCardStatus) => {
      if (!personalBoard) return;
      const now = new Date().toISOString();
      const optimistic: TaskBoard = {
        ...personalBoard,
        cards: personalBoard.cards.map(c =>
          c.id === card.id ? { ...c, status, updatedAt: now } : c
        ),
        updatedAt: now,
      };
      void mutatePersonal(
        optimistic,
        () => todosApi.updateStatus(USER_TASKS_THREAD_ID, card.id, status),
        personalBoard
      );
    },
    [personalBoard, mutatePersonal]
  );

  const handleUpdatePersonal = useCallback(
    (card: TaskBoardCard, nextCard: TaskBoardCard) => {
      if (!personalBoard) return;
      const now = new Date().toISOString();
      const optimistic: TaskBoard = {
        ...personalBoard,
        cards: personalBoard.cards.map(c =>
          c.id === card.id ? { ...nextCard, updatedAt: now } : c
        ),
        updatedAt: now,
      };
      void mutatePersonal(
        optimistic,
        () =>
          todosApi.edit({
            threadId: USER_TASKS_THREAD_ID,
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
          }),
        personalBoard
      );
    },
    [personalBoard, mutatePersonal]
  );

  const handleDeletePersonal = useCallback(
    (card: TaskBoardCard) => {
      if (!personalBoard) return;
      const optimistic: TaskBoard = {
        ...personalBoard,
        cards: personalBoard.cards.filter(c => c.id !== card.id),
        updatedAt: new Date().toISOString(),
      };
      void mutatePersonal(
        optimistic,
        () => todosApi.remove(USER_TASKS_THREAD_ID, card.id),
        personalBoard
      );
    },
    [personalBoard, mutatePersonal]
  );

  // ── derived agent board list (read-only) ─────────────────────────────

  const threadMap = new Map(threads.map(th => [th.id, th]));
  const allThreadIds = new Set([...Object.keys(liveBoards), ...Object.keys(persistedBoards)]);

  const boardEntries: ThreadTaskBoard[] = [];
  for (const threadId of allThreadIds) {
    if (threadId === USER_TASKS_THREAD_ID) continue; // personal board rendered separately
    const liveBoard = liveBoards[threadId];
    const persistedBoard = persistedBoards[threadId];
    const board = liveBoard ?? persistedBoard;
    if (!board || board.cards.length === 0) continue;

    const thread = threadMap.get(threadId);
    const title =
      thread?.title && thread.title.trim().length > 0
        ? thread.title
        : `${t('intelligence.tasks.threadPrefix')} ${shortId(threadId)}`;

    boardEntries.push({ threadId, title, board, live: Boolean(liveBoard) });
  }

  boardEntries.sort((a, b) => {
    if (a.live !== b.live) return a.live ? -1 : 1;
    return b.board.updatedAt.localeCompare(a.board.updatedAt);
  });

  const personalCards = personalBoard?.cards ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs text-stone-400 dark:text-neutral-500">
          {t('intelligence.tasks.subtitle')}
        </p>
        <button
          type="button"
          onClick={() => setComposerOpen(true)}
          className="inline-flex flex-none items-center gap-1.5 rounded-md bg-ocean-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-ocean-700">
          <LuPlus className="h-3.5 w-3.5" />
          {t('intelligence.tasks.newTask')}
        </button>
      </div>

      {actionError && (
        <div className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {actionError}
        </div>
      )}

      {/* Personal board — always present so users can manage their own tasks. */}
      <section className="space-y-2">
        <div className="flex items-center gap-2">
          <h3 className="truncate text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('intelligence.tasks.personalBoardTitle')}
          </h3>
        </div>
        {personalCards.length > 0 ? (
          <TaskKanbanBoard
            board={personalBoard as TaskBoard}
            hideHeader
            onMove={handleMovePersonal}
            onUpdateCard={handleUpdatePersonal}
            onDeleteCard={handleDeletePersonal}
          />
        ) : (
          <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-stone-200 dark:border-neutral-800 py-8 text-center text-stone-400 dark:text-neutral-500">
            <p className="text-sm font-medium">{t('intelligence.tasks.personalEmpty')}</p>
            <button
              type="button"
              onClick={() => setComposerOpen(true)}
              className="inline-flex items-center gap-1.5 text-xs font-medium text-ocean-600 hover:text-ocean-700 dark:text-ocean-300 dark:hover:text-ocean-200">
              <LuPlus className="h-3.5 w-3.5" />
              {t('intelligence.tasks.newTask')}
            </button>
          </div>
        )}
      </section>

      {loading && (
        <div className="flex items-center justify-center py-6 text-stone-400 dark:text-neutral-500">
          <div className="h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent mr-2" />
          <span className="text-sm">{t('intelligence.tasks.loadingBoards')}</span>
        </div>
      )}

      {error && (
        <div className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
          {t('intelligence.tasks.failedToLoad')}: {error}
        </div>
      )}

      {/* Agent / conversation boards — read-only. */}
      {boardEntries.map(entry => (
        <section key={entry.threadId} className="space-y-2">
          <div className="flex items-center gap-2">
            <h3
              className="truncate text-sm font-semibold text-stone-700 dark:text-neutral-200"
              title={entry.title}>
              {entry.title}
            </h3>
            {entry.live && (
              <span className="flex items-center gap-1 rounded-full border border-ocean-200 bg-ocean-50 px-2 py-0.5 text-[10px] font-medium text-ocean-600">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-ocean-500" />
                {t('intelligence.tasks.live')}
              </span>
            )}
          </div>

          <TaskKanbanBoard board={entry.board} hideHeader />
        </section>
      ))}

      {composerOpen && (
        <UserTaskComposer onCreated={handleCreated} onClose={() => setComposerOpen(false)} />
      )}
    </div>
  );
}
