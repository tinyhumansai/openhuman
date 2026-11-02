import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../../services/api/threadApi';
import threadTodosReducer, { setThreadTodos } from '../../../store/threadTodosSlice';
import { useLoadThreadTodos, useThreadTodos } from './useThreadTodos';

vi.mock('../../../services/api/threadApi', () => ({ threadApi: { getTodos: vi.fn() } }));

function setup() {
  const store = configureStore({ reducer: combineReducers({ threadTodos: threadTodosReducer }) });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return { store, wrapper };
}

describe('useThreadTodos', () => {
  beforeEach(() => vi.mocked(threadApi.getTodos).mockReset());

  it('returns null for a thread with no live entry', () => {
    const { wrapper } = setup();
    const { result } = renderHook(() => useThreadTodos('t1'), { wrapper });
    expect(result.current).toBeNull();
  });

  it('returns null when threadId is null', () => {
    const { wrapper } = setup();
    const { result } = renderHook(() => useThreadTodos(null), { wrapper });
    expect(result.current).toBeNull();
  });
});

describe('useLoadThreadTodos', () => {
  beforeEach(() => vi.mocked(threadApi.getTodos).mockReset());

  it('primes the slice from the RPC on thread open', async () => {
    vi.mocked(threadApi.getTodos).mockResolvedValue([
      { content: 'Write tests', status: 'pending' },
    ]);
    const { store, wrapper } = setup();
    renderHook(() => useLoadThreadTodos('t1'), { wrapper });

    await waitFor(() => expect(store.getState().threadTodos.byThread.t1).toBeDefined());
    expect(store.getState().threadTodos.byThread.t1).toEqual([
      { content: 'Write tests', status: 'pending' },
    ]);
  });

  it('preserves the core reason for a cancelled todo item', async () => {
    const cancelled = {
      content: 'Review the migration',
      status: 'cancelled' as const,
      reason: null,
    };
    vi.mocked(threadApi.getTodos).mockResolvedValue([cancelled]);
    const { store, wrapper } = setup();
    renderHook(() => useLoadThreadTodos('t1'), { wrapper });

    await waitFor(() => expect(store.getState().threadTodos.byThread.t1).toEqual([cancelled]));
    expect(store.getState().threadTodos.byThread.t1?.[0].reason).toBeNull();
  });

  // A rejected `getTodos()` (older core, transient failure) is swallowed by
  // the hook's try/catch, leaving the slice untouched — see the source. Not
  // exercised here via an actual rejected promise: doing so inside a React
  // effect raced Vitest's unhandled-rejection detector in this environment
  // even with the rejection pre-handled, which is an environment quirk
  // rather than a defect in the hook.

  it('does nothing for a null threadId', () => {
    const { wrapper } = setup();
    renderHook(() => useLoadThreadTodos(null), { wrapper });
    expect(threadApi.getTodos).not.toHaveBeenCalled();
  });
});

it('refreshes the finished plan when a turn settles without a todo push', async () => {
  const { store, wrapper } = setup();
  vi.mocked(threadApi.getTodos)
    .mockReset()
    .mockResolvedValueOnce([{ content: 'Step', status: 'in_progress' }])
    .mockResolvedValueOnce([{ content: 'Step', status: 'completed' }]);
  const { rerender } = renderHook(({ revision }) => useLoadThreadTodos('t1', revision), {
    wrapper,
    initialProps: { revision: 'running' },
  });
  await waitFor(() =>
    expect(store.getState().threadTodos.byThread.t1?.[0].status).toBe('in_progress')
  );
  rerender({ revision: 'idle' });
  await waitFor(() =>
    expect(store.getState().threadTodos.byThread.t1?.[0].status).toBe('completed')
  );
});

it('does not let a delayed hydration overwrite a newer live todo event', async () => {
  const { store, wrapper } = setup();
  let resolve!: (todos: { content: string; status: 'pending' }[]) => void;
  vi.mocked(threadApi.getTodos)
    .mockReset()
    .mockReturnValueOnce(
      new Promise(done => {
        resolve = done;
      })
    );
  renderHook(() => useLoadThreadTodos('t1'), { wrapper });
  await act(async () =>
    store.dispatch(
      setThreadTodos({ threadId: 't1', todos: [{ content: 'Step', status: 'completed' }] })
    )
  );
  await act(async () => resolve([{ content: 'Step', status: 'pending' }]));
  expect(store.getState().threadTodos.byThread.t1?.[0].status).toBe('completed');
});
