/**
 * IntelligenceTasksTab — shows all agent task boards across conversations.
 *
 * Aggregates:
 *  1. Live boards from `chatRuntime.taskBoardByThread` (updated in real-time
 *     while a conversation is running via socket events).
 *  2. Persisted boards fetched once on mount from `threadApi.listTurnStates`
 *     (each turn state may carry a saved `taskBoard`).
 *
 * Thread titles are resolved from `thread.threads` when available. Boards
 * from threads not present in the list fall back to a shortened thread id.
 *
 * This component is read-only — moves are not surfaced here; the user manages
 * task cards from the Conversations page where the write path lives.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { TaskKanbanBoard } from '../../pages/conversations/components/TaskKanbanBoard';
import { threadApi } from '../../services/api/threadApi';
import { useAppSelector } from '../../store/hooks';
import type { TaskBoard } from '../../types/turnState';

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
  const liveBoards = useAppSelector(state => state.chatRuntime.taskBoardByThread);
  const threads = useAppSelector(state => state.thread.threads ?? []);

  const [persistedBoards, setPersistedBoards] = useState<Record<string, TaskBoard>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const fetchPersistedBoards = useCallback(async () => {
    log('fetchPersistedBoards: entry');
    setLoading(true);
    setError(null);
    try {
      const turnStates = await threadApi.listTurnStates();
      log('fetchPersistedBoards: received %d turn states', turnStates.length);
      const boards: Record<string, TaskBoard> = {};
      for (const ts of turnStates) {
        if (ts.taskBoard && ts.taskBoard.cards.length > 0) {
          boards[ts.threadId] = ts.taskBoard;
          log(
            'fetchPersistedBoards: board threadId=%s cards=%d',
            ts.threadId,
            ts.taskBoard.cards.length
          );
        }
      }
      if (mountedRef.current) {
        setPersistedBoards(boards);
        log('fetchPersistedBoards: done boards=%d', Object.keys(boards).length);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('fetchPersistedBoards: error %s', msg);
      if (mountedRef.current) {
        setError(msg);
      }
    } finally {
      if (mountedRef.current) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    fetchPersistedBoards();
    return () => {
      mountedRef.current = false;
    };
  }, [fetchPersistedBoards]);

  // Build the merged, deduplicated board list. Live boards take priority
  // over persisted ones for the same thread (they reflect the latest agent
  // turn in progress).
  const threadMap = new Map(threads.map(t => [t.id, t]));
  const allThreadIds = new Set([...Object.keys(liveBoards), ...Object.keys(persistedBoards)]);

  const boardEntries: ThreadTaskBoard[] = [];
  for (const threadId of allThreadIds) {
    const liveBoard = liveBoards[threadId];
    const persistedBoard = persistedBoards[threadId];
    const board = liveBoard ?? persistedBoard;
    if (!board || board.cards.length === 0) continue;

    const thread = threadMap.get(threadId);
    const title =
      thread?.title && thread.title.trim().length > 0
        ? thread.title
        : `Thread ${shortId(threadId)}`;

    boardEntries.push({ threadId, title, board, live: Boolean(liveBoard) });
  }

  // Sort: live boards first, then by most-recently-updated.
  boardEntries.sort((a, b) => {
    if (a.live !== b.live) return a.live ? -1 : 1;
    return b.board.updatedAt.localeCompare(a.board.updatedAt);
  });

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12 text-stone-400">
        <div className="h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent mr-2" />
        <span className="text-sm">Loading task boards…</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-xl border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700">
        Failed to load task boards: {error}
      </div>
    );
  }

  if (boardEntries.length === 0) {
    return (
      <div className="flex flex-col items-center gap-3 py-12 text-center text-stone-400">
        <div className="text-3xl">📋</div>
        <p className="text-sm font-medium">No agent task boards yet</p>
        <p className="text-xs text-stone-400">
          Start a conversation and ask the agent to manage tasks — boards will appear here as cards
          are created.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <p className="text-xs text-stone-400">
        {boardEntries.length} active board{boardEntries.length !== 1 ? 's' : ''} across
        conversations
      </p>
      {boardEntries.map(entry => (
        <section key={entry.threadId} className="space-y-2">
          <div className="flex items-center gap-2">
            <h3 className="truncate text-sm font-semibold text-stone-700" title={entry.title}>
              {entry.title}
            </h3>
            {entry.live && (
              <span className="flex items-center gap-1 rounded-full border border-ocean-200 bg-ocean-50 px-2 py-0.5 text-[10px] font-medium text-ocean-600">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-ocean-500" />
                live
              </span>
            )}
          </div>
          <TaskKanbanBoard board={entry.board} />
        </section>
      ))}
    </div>
  );
}
