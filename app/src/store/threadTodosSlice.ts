/**
 * The LIVE thread-level todo list, driven by the core's `thread_todos_changed`
 * socket event (and the `openhuman.threads_todos_get` RPC on thread open /
 * reconnect) — not by scraping the deprecated `todo` tool-result payload out
 * of the timeline (see the old `harnessState.ts`, which this replaces).
 *
 * Kept as its own small slice rather than folded into `chatRuntimeSlice`
 * (which already owns the tool timeline, approvals, and plan review) because
 * this state has an entirely different source of truth: a dedicated core
 * event/RPC pair, not tool-call bookkeeping.
 */
import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { TodoItem as TodoItemWire } from '../types/threadWire.generated';

/** Core wire status for one todo item (`DomainEvent::ThreadTodosChanged`). */
export type CoreTodoStatus = TodoItemWire['status'];

/** One todo item as the core sends it — wire shape, not the element's shape. */
export type ThreadTodoItemView = TodoItemWire;

export interface ThreadTodosState {
  byThread: Record<string, ThreadTodoItemView[]>;
}

const initialState: ThreadTodosState = { byThread: {} };

const threadTodosSlice = createSlice({
  name: 'threadTodos',
  initialState,
  reducers: {
    setThreadTodos: (
      state,
      action: PayloadAction<{ threadId: string; todos: ThreadTodoItemView[] }>
    ) => {
      state.byThread[action.payload.threadId] = action.payload.todos;
    },
    clearThreadTodos: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.byThread[action.payload.threadId];
    },
  },
});

export const { setThreadTodos, clearThreadTodos } = threadTodosSlice.actions;
export default threadTodosSlice.reducer;
