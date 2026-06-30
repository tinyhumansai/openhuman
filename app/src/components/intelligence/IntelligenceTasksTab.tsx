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
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  LuArrowRight,
  LuBot,
  LuCheck,
  LuExternalLink,
  LuPlus,
  LuSparkles,
  LuX,
} from 'react-icons/lu';
import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { TaskKanbanBoard } from '../../pages/conversations/components/TaskKanbanBoard';
import { isTaskThread } from '../../pages/conversations/utils/threadFilter';
import { threadApi } from '../../services/api/threadApi';
import {
  TASK_SOURCES_THREAD_ID,
  todosApi,
  USER_TASKS_THREAD_ID,
} from '../../services/api/todosApi';
import { chatSend } from '../../services/chatService';
import { selectActiveAgentProfileId } from '../../store/agentProfileSlice';
import { beginInferenceTurn, setToolTimelineForThread } from '../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  loadThreadMessages,
  loadThreads,
  setActiveThread,
  setSelectedThread,
} from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import type { TaskBoard, TaskBoardCard, TaskBoardCardStatus } from '../../types/turnState';
import { chatThreadPath } from '../../utils/chatRoutes';
import { settingsNavState } from '../settings/modal/settingsOverlay';
import Button from '../ui/Button';
import { UserTaskComposer } from './UserTaskComposer';

const log = debug('intelligence:tasks');
const AGENT_TASK_THREAD_LABEL = 'tasks';
const CHAT_MODEL_ID = 'reasoning-v1';

interface ThreadTaskBoard {
  threadId: string;
  title: string;
  board: TaskBoard;
  live: boolean;
}

function shortId(threadId: string): string {
  return threadId.length > 8 ? `…${threadId.slice(-8)}` : threadId;
}

function agentTaskThreadTitle(title: string): string {
  const trimmed = title.trim();
  const base = trimmed.length > 72 ? `${trimmed.slice(0, 69)}...` : trimmed;
  return `Agent task: ${base || 'Untitled task'}`;
}

function buildAgentTaskPrompt(
  card: TaskBoardCard,
  t: (key: string, fallback?: string) => string
): string {
  const lines: string[] = [
    'Work this approved agent task from the task board.',
    '',
    `Task: ${card.title}`,
  ];
  if (card.objective?.trim()) {
    lines.push('', 'Objective:', card.objective.trim());
  }
  if (card.notes?.trim()) {
    lines.push('', 'Notes:', card.notes.trim());
  }
  if (card.plan && card.plan.length > 0) {
    lines.push('', 'Plan:');
    lines.push(...card.plan.map((step, index) => `${index + 1}. ${step}`));
  }
  if (card.acceptanceCriteria && card.acceptanceCriteria.length > 0) {
    lines.push('', 'Acceptance criteria:');
    lines.push(...card.acceptanceCriteria.map(item => `- ${item}`));
  }
  if (card.allowedTools && card.allowedTools.length > 0) {
    lines.push('', `Allowed tools: ${card.allowedTools.join(', ')}`);
  }
  if (card.evidence && card.evidence.length > 0) {
    lines.push('', 'Evidence / references:');
    lines.push(...card.evidence.map(item => `- ${item}`));
  }
  const source = readSourceMetadata(card.sourceMetadata);
  if (source.url || source.repo || source.externalId) {
    lines.push('', t('intelligence.workTask.sourceTaskHeading'));
    if (source.repo)
      lines.push(t('intelligence.workTask.repositoryLine').replace('{repo}', source.repo));
    if (source.externalId)
      lines.push(
        t('intelligence.workTask.externalIdLine').replace('{externalId}', source.externalId)
      );
    if (source.url) lines.push(t('intelligence.workTask.urlLine').replace('{url}', source.url));
  }
  lines.push('', t('intelligence.workTask.closingInstruction'));
  return lines.join('\n');
}

export default function IntelligenceTasksTab() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const liveBoards = useAppSelector(state => state.chatRuntime.taskBoardByThread);
  const threads = useAppSelector(state => state.thread.threads ?? []);
  const selectedAgentProfileId = useAppSelector(selectActiveAgentProfileId);
  const uiLocale = useAppSelector(state => state.locale?.current ?? 'en');

  const [persistedBoards, setPersistedBoards] = useState<Record<string, TaskBoard>>({});
  const [personalBoard, setPersonalBoard] = useState<TaskBoard | null>(null);
  const [taskSourcesBoard, setTaskSourcesBoard] = useState<TaskBoard | null>(null);
  const [composerOpen, setComposerOpen] = useState(false);
  const [refiningCard, setRefiningCard] = useState<TaskBoardCard | null>(null);
  const [workingCardId, setWorkingCardId] = useState<string | null>(null);
  const [mutatingCardId, setMutatingCardId] = useState<string | null>(null);
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

  const fetchTaskSourcesBoard = useCallback(async () => {
    log('fetchTaskSourcesBoard: entry');
    try {
      const board = await todosApi.list(TASK_SOURCES_THREAD_ID);
      if (mountedRef.current) {
        setTaskSourcesBoard(board);
        log('fetchTaskSourcesBoard: cards=%d', board.cards.length);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('fetchTaskSourcesBoard: error %s', msg);
      if (mountedRef.current) {
        setTaskSourcesBoard({ threadId: TASK_SOURCES_THREAD_ID, cards: [], updatedAt: '' });
      }
    }
  }, []);

  const loadAll = useCallback(async () => {
    // `loading` defaults to true; flip it off once both fetches settle.
    await Promise.allSettled([
      fetchPersistedBoards(),
      fetchPersonalBoard(),
      fetchTaskSourcesBoard(),
    ]);
    if (mountedRef.current) setLoading(false);
  }, [fetchPersistedBoards, fetchPersonalBoard, fetchTaskSourcesBoard]);

  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => {
      void loadAll();
    }, 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [loadAll]);

  // Background board mutations — the dispatcher poller claiming a card,
  // `update_task` from an autonomous run, `write_back`, triage, stale-reclaim —
  // update the persisted board but emit no live socket event to this tab (the
  // progress→socket bridge only fires inside an interactive, client-connected
  // turn). So the user-tasks + task-sources boards would otherwise look frozen
  // while work happens in the background. Re-read just those two on a light
  // interval while the tab is visible; they're local in-process RPC, so it's
  // cheap. (A push-based fix would need a core DomainEvent + socket broadcast;
  // this keeps the fix isolated to the tab and catches every mutation source.)
  useEffect(() => {
    const POLL_MS = 4000;
    const tick = () => {
      if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return;
      void fetchPersonalBoard();
      void fetchTaskSourcesBoard();
    };
    const handle = window.setInterval(tick, POLL_MS);
    return () => window.clearInterval(handle);
  }, [fetchPersonalBoard, fetchTaskSourcesBoard]);

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
    async (
      optimistic: TaskBoard,
      call: () => Promise<TaskBoard>,
      previous: TaskBoard,
      cardId?: string
    ) => {
      setActionError(null);
      if (cardId && mountedRef.current) setMutatingCardId(cardId);
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
      } finally {
        if (mountedRef.current) setMutatingCardId(null);
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
        personalBoard,
        card.id
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
        personalBoard,
        card.id
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
        personalBoard,
        card.id
      );
    },
    [personalBoard, mutatePersonal]
  );

  const handleWorkPersonal = useCallback(
    async (card: TaskBoardCard) => {
      if (!personalBoard || workingCardId) return;
      setWorkingCardId(card.id);
      setActionError(null);
      const launchPrompt = buildAgentTaskPrompt(card, t);
      const now = new Date().toISOString();
      const threadTitle = agentTaskThreadTitle(card.title);
      try {
        const thread = await threadApi.createNewThread([AGENT_TASK_THREAD_LABEL]);
        await threadApi.updateTitle(thread.id, threadTitle);
        const userMessage: ThreadMessage = {
          id: `msg_${globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `${Date.now()}`}`,
          content: launchPrompt,
          type: 'text',
          extraMetadata: { source: 'agent-task-board', taskCardId: card.id },
          sender: 'user',
          createdAt: now,
        };
        await threadApi.appendMessage(thread.id, userMessage);

        const startedBoard = await todosApi.updateStatus(
          USER_TASKS_THREAD_ID,
          card.id,
          'in_progress'
        );
        // Link the card to its session thread so the board offers "View session".
        const linkedBoard = await todosApi
          .setSessionThread(USER_TASKS_THREAD_ID, card.id, thread.id)
          .catch(() => startedBoard);
        if (mountedRef.current) setPersonalBoard(linkedBoard);

        dispatch(setSelectedThread(thread.id));
        dispatch(setToolTimelineForThread({ threadId: thread.id, entries: [] }));
        dispatch(beginInferenceTurn({ threadId: thread.id }));
        dispatch(setActiveThread(thread.id));
        void dispatch(loadThreads());
        void dispatch(loadThreadMessages(thread.id));
        navigate(chatThreadPath(thread.id));

        await chatSend({
          threadId: thread.id,
          message: launchPrompt,
          model: CHAT_MODEL_ID,
          profileId: selectedAgentProfileId,
          locale: uiLocale,
        });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('work personal task failed: %s', msg);
        if (mountedRef.current) setActionError(t('intelligence.tasks.workTaskFailed'));
      } finally {
        if (mountedRef.current) setWorkingCardId(null);
      }
    },
    [dispatch, navigate, personalBoard, selectedAgentProfileId, t, uiLocale, workingCardId]
  );

  const handleApproveSourcePlan = useCallback(
    async (sourceCard: TaskBoardCard, draft: RefinedTaskDraft) => {
      const now = new Date().toISOString();
      setActionError(null);
      try {
        const sourceExternalId = readSourceMetadata(sourceCard.sourceMetadata).externalId;
        // Idempotent approve: if this source item was already promoted to
        // user-tasks (picked up / running), don't add a second card — just
        // retire the inbox card. Stops the duplicate when an edited item is
        // re-offered for approval.
        const alreadyPromoted = sourceExternalId
          ? personalBoard?.cards.some(
              c => readSourceMetadata(c.sourceMetadata).externalId === sourceExternalId
            )
          : false;
        if (alreadyPromoted) {
          const sourceSaved = await todosApi.updateStatus(
            TASK_SOURCES_THREAD_ID,
            sourceCard.id,
            'done'
          );
          if (mountedRef.current) {
            setTaskSourcesBoard(sourceSaved);
            setRefiningCard(null);
          }
          return;
        }
        const added = await todosApi.add({
          threadId: USER_TASKS_THREAD_ID,
          content: draft.title,
          status: 'todo',
          objective: draft.objective,
          notes: draft.notes,
          // Stamp the source link so the inbox can detect it's now picked up.
          sourceMetadata: sourceCard.sourceMetadata,
        });
        const created =
          added.cards.find(card => card.title === draft.title && card.updatedAt >= now) ??
          added.cards[added.cards.length - 1];
        const saved = created
          ? await todosApi.edit({
              threadId: USER_TASKS_THREAD_ID,
              id: created.id,
              content: draft.title,
              status: 'todo',
              objective: draft.objective,
              notes: draft.notes,
              assignedAgent: 'orchestrator',
              approvalMode: 'not_required',
              plan: draft.plan,
              allowedTools: draft.allowedTools,
              acceptanceCriteria: draft.acceptanceCriteria,
              evidence: draft.evidence,
            })
          : added;
        if (mountedRef.current) {
          setPersonalBoard(saved);
        }

        const sourceSaved = await todosApi.updateStatus(
          TASK_SOURCES_THREAD_ID,
          sourceCard.id,
          'done'
        );
        if (mountedRef.current) {
          setTaskSourcesBoard(sourceSaved);
          setRefiningCard(null);
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('source task approval failed: %s', msg);
        if (mountedRef.current) setActionError(t('intelligence.tasks.sourcePlan.createFailed'));
      }
    },
    [t, personalBoard]
  );

  // ── derived agent board list (read-only) ─────────────────────────────

  const threadMap = new Map(threads.map(th => [th.id, th]));
  const allThreadIds = new Set([...Object.keys(liveBoards), ...Object.keys(persistedBoards)]);

  // Hide task-source items already picked up (promoted to the user-tasks board)
  // so an already-running task isn't re-offered for approval in the inbox.
  const pickedUpExternalIds = new Set(
    (personalBoard?.cards ?? [])
      .map(c => readSourceMetadata(c.sourceMetadata).externalId)
      .filter((id): id is string => Boolean(id))
  );
  const visibleTaskSourcesBoard: TaskBoard | null = taskSourcesBoard
    ? {
        ...taskSourcesBoard,
        cards: taskSourcesBoard.cards.filter(
          c => !pickedUpExternalIds.has(readSourceMetadata(c.sourceMetadata).externalId ?? '')
        ),
      }
    : null;

  const boardEntries: ThreadTaskBoard[] = [];
  for (const threadId of allThreadIds) {
    if (threadId === USER_TASKS_THREAD_ID) continue; // personal board rendered separately
    if (threadId === TASK_SOURCES_THREAD_ID) continue; // task sources rendered separately
    const thread = threadMap.get(threadId);
    // Skip task SESSION threads (autonomous `task-*` runs + manual task-labelled
    // threads): their cards already appear on the user-tasks / task-sources
    // boards and the session is reachable via the card's "View work" — rendering
    // their live board here just duplicates those cards as redundant tables.
    if (threadId.startsWith('task-') || (thread && isTaskThread(thread))) continue;
    const liveBoard = liveBoards[threadId];
    const persistedBoard = persistedBoards[threadId];
    const board = liveBoard ?? persistedBoard;
    // Show live agent boards even with 0 cards so users can see activity in progress.
    if (!board || (board.cards.length === 0 && !liveBoard)) continue;
    const title =
      thread?.title && thread.title.trim().length > 0
        ? thread.title
        : t('intelligence.tasks.threadPrefix').replace('{thread}', shortId(threadId));

    boardEntries.push({ threadId, title, board, live: Boolean(liveBoard) });
  }

  boardEntries.sort((a, b) => {
    if (a.live !== b.live) return a.live ? -1 : 1;
    return b.board.updatedAt.localeCompare(a.board.updatedAt);
  });

  // personalCards kept for reference — board always rendered now, empty state handled inside TaskKanbanBoard
  // const personalCards = personalBoard?.cards ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs text-content-faint">{t('intelligence.tasks.subtitle')}</p>
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
          <h3 className="truncate text-sm font-semibold text-content-secondary">
            {t('intelligence.tasks.personalBoardTitle')}
          </h3>
        </div>
        <TaskKanbanBoard
          board={personalBoard ?? { threadId: USER_TASKS_THREAD_ID, cards: [], updatedAt: '' }}
          hideHeader
          onMove={handleMovePersonal}
          onUpdateCard={handleUpdatePersonal}
          onDeleteCard={handleDeletePersonal}
          onWorkTask={handleWorkPersonal}
          onViewSession={card => {
            if (!card.sessionThreadId) return;
            const tid = card.sessionThreadId;
            // Open the exact session — mirror the manual "Work" path's
            // thread-open sequence so /chat lands on this thread, not just
            // the Conversations page.
            // Navigation only — do NOT mark the thread active. activeThreadId
            // tracks a true in-flight turn; a completed session never emits the
            // done/error lifecycle that would clear it, so forcing it active
            // would wedge the composer until then.
            dispatch(setSelectedThread(tid));
            void dispatch(loadThreads());
            void dispatch(loadThreadMessages(tid));
            navigate(chatThreadPath(tid));
          }}
          workingCardId={workingCardId}
          mutatingCardId={mutatingCardId}
        />
      </section>

      {taskSourcesBoard && (
        <section className="space-y-2">
          <TaskSourceTaskList
            board={visibleTaskSourcesBoard ?? taskSourcesBoard}
            disabled={loading}
            onWorkOnTask={setRefiningCard}
          />
        </section>
      )}

      {loading && (
        <div className="flex items-center justify-center py-6 text-content-faint">
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
              className="truncate text-sm font-semibold text-content-secondary"
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
      {refiningCard && (
        <TaskSourceRefinementDialog
          card={refiningCard}
          disabled={loading}
          onClose={() => setRefiningCard(null)}
          onApprove={handleApproveSourcePlan}
        />
      )}
    </div>
  );
}

interface SourceMetadata {
  provider?: string;
  externalId?: string;
  url?: string;
  repo?: string;
  urgency?: number;
}

interface RefinedTaskDraft {
  title: string;
  objective: string;
  notes: string;
  plan: string[];
  allowedTools: string[];
  acceptanceCriteria: string[];
  evidence: string[];
}

function readSourceMetadata(value: Record<string, unknown> | null | undefined): SourceMetadata {
  if (!value) return {};
  return {
    provider: readString(value.provider),
    externalId: readString(value.external_id) ?? readString(value.externalId),
    url: readString(value.url),
    repo: readString(value.repo),
    urgency: readNumber(value.urgency),
  };
}

function readString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function readNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

function sourceProviderLabel(provider: string | undefined, t: (key: string) => string): string {
  switch (provider) {
    case 'github':
      return t('settings.taskSources.providers.github');
    case 'notion':
      return t('settings.taskSources.providers.notion');
    case 'linear':
      return t('settings.taskSources.providers.linear');
    case 'clickup':
      return t('settings.taskSources.providers.clickup');
    default:
      return provider ?? t('conversations.taskKanban.source.unknownProvider');
  }
}

function taskSourceLabel(card: TaskBoardCard, t: (key: string) => string): string {
  const source = readSourceMetadata(card.sourceMetadata);
  const provider = sourceProviderLabel(source.provider, t);
  if (source.repo && source.externalId) return `${provider} · ${source.repo}#${source.externalId}`;
  if (source.externalId) return `${provider} · ${source.externalId}`;
  return provider;
}

function buildRefinedDraft(
  card: TaskBoardCard,
  t: (key: string, fallback?: string) => string
): RefinedTaskDraft {
  const source = readSourceMetadata(card.sourceMetadata);
  const objective =
    card.objective?.trim() ||
    t('intelligence.refine.objectiveDefault').replace('{title}', card.title);
  const sourceLine = source.url
    ? t('intelligence.refine.sourceLine').replace('{url}', source.url)
    : t('intelligence.refine.sourceIntake');
  const repoLine = source.repo
    ? t('intelligence.refine.repositoryLine').replace('{repo}', source.repo)
    : null;
  const externalLine = source.externalId
    ? t('intelligence.refine.externalTaskLine').replace('{externalId}', source.externalId)
    : null;

  return {
    title: card.title.replace(/^GitHub:\s*/i, '').trim() || card.title,
    objective,
    notes: [card.notes?.trim(), sourceLine, repoLine, externalLine].filter(Boolean).join('\n'),
    plan:
      card.plan && card.plan.length > 0
        ? card.plan
        : [
            t('intelligence.refine.planStep1'),
            t('intelligence.refine.planStep2'),
            t('intelligence.refine.planStep3'),
            t('intelligence.refine.planStep4'),
          ],
    allowedTools:
      card.allowedTools && card.allowedTools.length > 0
        ? card.allowedTools
        : ['code_search', 'shell', 'edit', 'tests'],
    acceptanceCriteria:
      card.acceptanceCriteria && card.acceptanceCriteria.length > 0
        ? card.acceptanceCriteria
        : [
            t('intelligence.refine.acceptance1'),
            t('intelligence.refine.acceptance2'),
            t('intelligence.refine.acceptance3'),
          ],
    evidence:
      card.evidence && card.evidence.length > 0 ? card.evidence : source.url ? [source.url] : [],
  };
}

function TaskSourceTaskList({
  board,
  disabled,
  onWorkOnTask,
}: {
  board: TaskBoard;
  disabled: boolean;
  onWorkOnTask: (card: TaskBoardCard) => void;
}) {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const sortedCards = useMemo(
    () => [...board.cards].sort((a, b) => a.order - b.order),
    [board.cards]
  );

  return (
    <section className="space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-content-secondary">
            {t('settings.taskSources.title')}
          </h3>
          <p className="text-xs text-content-faint">
            {t('intelligence.tasks.sourceList.subtitle')}
          </p>
        </div>
        <button
          type="button"
          onClick={() => navigate('/settings/integrations', settingsNavState(location))}
          className="text-xs font-medium text-ocean-600 hover:text-ocean-700 dark:text-ocean-300 dark:hover:text-ocean-200">
          {t('conversations.taskKanban.sources.manage')}
        </button>
      </div>

      {sortedCards.length === 0 ? (
        <div className="rounded-xl border border-dashed border-line py-7 text-center text-sm text-content-faint">
          {t('intelligence.tasks.sourceList.empty')}
        </div>
      ) : (
        <ul className="divide-y divide-line-subtle rounded-xl border border-line bg-surface dark:divide-neutral-800">
          {sortedCards.map(card => {
            const source = readSourceMetadata(card.sourceMetadata);
            const done = card.status === 'done';
            return (
              <li key={card.id} className="p-3">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="rounded-md bg-sky-50 px-1.5 py-0.5 text-[10px] font-medium text-sky-700 dark:bg-sky-500/10 dark:text-sky-200">
                        {taskSourceLabel(card, t)}
                      </span>
                      {done && (
                        <span className="inline-flex items-center gap-1 rounded-md bg-sage-50 px-1.5 py-0.5 text-[10px] font-medium text-sage-700 dark:bg-sage-500/10 dark:text-sage-200">
                          <LuCheck className="h-3 w-3" />
                          {t('intelligence.tasks.sourceList.queued')}
                        </span>
                      )}
                    </div>
                    <p className="break-words text-sm font-medium leading-snug text-content">
                      {card.title}
                    </p>
                    {(card.objective || card.notes) && (
                      <p className="line-clamp-2 break-words text-xs leading-snug text-content-muted">
                        {card.objective || card.notes}
                      </p>
                    )}
                  </div>
                  <div className="flex flex-none items-center gap-2">
                    {source.url && (
                      <a
                        href={source.url}
                        target="_blank"
                        rel="noreferrer"
                        title={t('conversations.taskKanban.source.openExternal')}
                        className="flex h-8 w-8 items-center justify-center rounded-md border border-line text-content-muted hover:bg-surface-hover dark:text-content-secondary">
                        <LuExternalLink className="h-4 w-4" />
                      </a>
                    )}
                    <button
                      type="button"
                      disabled={disabled}
                      onClick={() => onWorkOnTask(card)}
                      className="inline-flex items-center gap-1.5 rounded-md bg-ocean-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-ocean-700 disabled:opacity-40">
                      <LuSparkles className="h-3.5 w-3.5" />
                      {t('intelligence.tasks.sourceList.workOnTask')}
                    </button>
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function TaskSourceRefinementDialog({
  card,
  disabled,
  onClose,
  onApprove,
}: {
  card: TaskBoardCard;
  disabled: boolean;
  onClose: () => void;
  onApprove: (card: TaskBoardCard, draft: RefinedTaskDraft) => Promise<void>;
}) {
  const { t } = useT();
  const initialDraft = useMemo(() => buildRefinedDraft(card, t), [card, t]);
  const [title, setTitle] = useState(initialDraft.title);
  const [objective, setObjective] = useState(initialDraft.objective);
  const [notes, setNotes] = useState(initialDraft.notes);
  const [planText, setPlanText] = useState(initialDraft.plan.join('\n'));
  const [criteriaText, setCriteriaText] = useState(initialDraft.acceptanceCriteria.join('\n'));
  const [saving, setSaving] = useState(false);

  const approve = async () => {
    setSaving(true);
    try {
      await onApprove(card, {
        title: title.trim() || card.title,
        objective: objective.trim(),
        notes: notes.trim(),
        plan: lines(planText),
        allowedTools: initialDraft.allowedTools,
        acceptanceCriteria: lines(criteriaText),
        evidence: initialDraft.evidence,
      });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
      <div className="max-h-[90vh] w-full max-w-2xl overflow-hidden rounded-xl border border-line bg-surface shadow-xl dark:bg-surface-canvas">
        <div className="flex items-start justify-between gap-3 border-b border-line-subtle px-4 py-3">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-content">
              {t('intelligence.tasks.sourcePlan.title')}
            </h3>
            <p className="mt-0.5 text-xs text-content-muted">
              {t('intelligence.tasks.sourcePlan.subtitle')}
            </p>
          </div>
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            aria-label={t('common.close')}
            onClick={onClose}
            className="flex-none">
            <LuX className="h-4 w-4" />
          </Button>
        </div>

        <div className="max-h-[calc(90vh-8rem)] space-y-4 overflow-y-auto px-4 py-4">
          <div className="rounded-lg border border-sky-100 bg-sky-50 px-3 py-2 text-xs text-sky-800 dark:border-sky-500/20 dark:bg-sky-500/10 dark:text-sky-200">
            <div className="flex items-center gap-2 font-medium">
              <LuBot className="h-3.5 w-3.5" />
              {t('intelligence.tasks.sourcePlan.researchAgent')}
            </div>
          </div>

          <label className="block space-y-1">
            <span className="text-xs font-medium text-content-secondary">
              {t('conversations.taskKanban.field.title')}
            </span>
            <input
              value={title}
              onChange={event => setTitle(event.target.value)}
              className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content outline-none focus:border-ocean-400"
            />
          </label>

          <label className="block space-y-1">
            <span className="text-xs font-medium text-content-secondary">
              {t('conversations.taskKanban.field.objective')}
            </span>
            <textarea
              value={objective}
              onChange={event => setObjective(event.target.value)}
              rows={3}
              className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content outline-none focus:border-ocean-400"
            />
          </label>

          <label className="block space-y-1">
            <span className="text-xs font-medium text-content-secondary">
              {t('conversations.taskKanban.field.plan')}
            </span>
            <textarea
              value={planText}
              onChange={event => setPlanText(event.target.value)}
              rows={5}
              className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content outline-none focus:border-ocean-400"
            />
          </label>

          <label className="block space-y-1">
            <span className="text-xs font-medium text-content-secondary">
              {t('conversations.taskKanban.field.acceptanceCriteria')}
            </span>
            <textarea
              value={criteriaText}
              onChange={event => setCriteriaText(event.target.value)}
              rows={4}
              className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content outline-none focus:border-ocean-400"
            />
          </label>

          <label className="block space-y-1">
            <span className="text-xs font-medium text-content-secondary">
              {t('conversations.taskKanban.field.notes')}
            </span>
            <textarea
              value={notes}
              onChange={event => setNotes(event.target.value)}
              rows={3}
              className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content outline-none focus:border-ocean-400"
            />
          </label>
        </div>

        <div className="flex flex-wrap items-center justify-end gap-2 border-t border-line-subtle px-4 py-3">
          <Button variant="secondary" size="sm" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </Button>
          <button
            type="button"
            onClick={() => void approve()}
            disabled={disabled || saving}
            className="inline-flex items-center gap-1.5 rounded-md bg-ocean-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-ocean-700 disabled:opacity-40">
            <LuArrowRight className="h-3.5 w-3.5" />
            {saving
              ? t('intelligence.tasks.sourcePlan.creating')
              : t('intelligence.tasks.sourcePlan.approve')}
          </button>
        </div>
      </div>
    </div>
  );
}

function lines(value: string): string[] {
  return value
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean);
}
