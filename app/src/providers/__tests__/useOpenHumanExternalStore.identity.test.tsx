import { configureStore } from '@reduxjs/toolkit';
import { act, renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import threadReducer, { addMessageLocal } from '../../store/threadSlice';
import { useOpenHumanExternalStore } from '../useOpenHumanExternalStore';

vi.mock('../../services/api/threadApi', () => ({
  threadApi: { getDerivedTranscript: vi.fn().mockResolvedValue({ items: [], nextCursor: null }) },
}));

describe('assistant-ui thread identity', () => {
  it('projects the real thread id so native thread switching can reset the viewport', () => {
    const store = configureStore({
      reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer },
    });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );
    const { result, rerender } = renderHook(
      ({ threadId }: { threadId: string | null }) => useOpenHumanExternalStore(threadId),
      { wrapper, initialProps: { threadId: 'first' as string | null } }
    );
    expect(result.current.adapters).toMatchObject({ threadList: { threadId: 'first' } });
    rerender({ threadId: 'second' });
    expect(result.current.adapters).toMatchObject({ threadList: { threadId: 'second' } });
    rerender({ threadId: null });
    expect(result.current.adapters).toMatchObject({ threadList: { threadId: undefined } });
  });
});

it('keeps the native message converter stable when the next turn changes the adapter', async () => {
  const store = configureStore({
    reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer },
  });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  const { result } = renderHook(() => useOpenHumanExternalStore('test'), { wrapper });
  const convert = result.current.convertMessage;
  await act(async () =>
    store.dispatch({
      type: addMessageLocal.fulfilled.type,
      payload: {
        threadId: 'test',
        message: {
          id: 'try-now',
          sender: 'user',
          type: 'text',
          content: 'try now',
          createdAt: '2026-10-10T00:00:00Z',
          extraMetadata: {},
        },
      },
    })
  );
  expect(result.current.convertMessage).toBe(convert);
});
