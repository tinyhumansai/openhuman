/**
 * `onReload` — assistant-ui's Reload button on an assistant reply.
 *
 * assistant-ui calls `onReload(parentId, { parentId, sourceId })` where
 * `parentId` is the USER prompt before the reply and `sourceId` is the reply
 * itself. The core's `threads.regenerate` only accepts an assistant reply id
 * (`agent:<request_id>`) or no id at all (redo the last turn); sending the
 * user prompt's `msg_<uuid>` id was rejected with "is not a regenerable
 * assistant reply" and surfaced as an unhandled rejection (Sentry
 * TAURI-REACT-AJ/AP/9W).
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import threadReducer, { addInferenceResponse } from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import { useOpenHumanExternalStore } from '../useOpenHumanExternalStore';

const hoisted = vi.hoisted(() => ({ regenerate: vi.fn(), toastAdd: vi.fn() }));

vi.mock('../../services/chatService', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/chatService')>()),
  regenerateMessage: (...args: unknown[]) => hoisted.regenerate(...args),
}));

vi.mock('../../components/ui/Toast', () => ({
  toast: { add: (...args: unknown[]) => hoisted.toastAdd(...args) },
}));

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 't-reload',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));

const THREAD_ID = 't-reload';

function row(
  id: string,
  sender: 'user' | 'agent',
  extraMetadata: Record<string, unknown> = {}
): ThreadMessage {
  return {
    id,
    sender,
    type: 'text',
    content: `${sender} ${id}`,
    extraMetadata,
    createdAt: '2026-01-01T00:00:00.000Z',
  };
}

// Alternating user/agent rows so no two assistant rows merge into one bubble.
const messages: ThreadMessage[] = [
  row('msg_u1', 'user'),
  row('agent:req-1', 'agent', { requestId: 'req-1' }),
  row('msg_u2', 'user'),
  row('msg_a2', 'agent', { requestId: 'req-2' }),
  row('msg_u3', 'user'),
  row('msg_a3', 'agent'),
  row('msg_u4', 'user'),
  row('msg_a4', 'agent'),
];

function buildStore() {
  return configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        ...threadReducer(undefined, { type: '@@INIT' }),
        selectedThreadId: THREAD_ID,
        messagesByThreadId: { [THREAD_ID]: messages },
        messages,
      },
    } as never,
  });
}

function mountAdapter(store: ReturnType<typeof buildStore>) {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return renderHook(() => useOpenHumanExternalStore(THREAD_ID), { wrapper });
}

function reload(
  result: ReturnType<typeof mountAdapter>['result'],
  parentId: string,
  sourceId: string
) {
  const onReload = result.current.onReload as (
    parentId: string | null,
    config: { parentId: string | null; sourceId?: string | null }
  ) => Promise<void>;
  return onReload(parentId, { parentId, sourceId });
}

function cachedIds(store: ReturnType<typeof buildStore>) {
  return store.getState().thread.messagesByThreadId[THREAD_ID].map(m => m.id);
}

beforeEach(() => {
  hoisted.regenerate.mockReset();
  hoisted.regenerate.mockResolvedValue(undefined);
  hoisted.toastAdd.mockReset();
});

describe('onReload — regenerating an assistant reply', () => {
  it('sends the agent: reply id from sourceId, never the user prompt id', async () => {
    const store = buildStore();
    const { result } = mountAdapter(store);

    await reload(result, 'msg_u1', 'agent:req-1');

    expect(hoisted.regenerate).toHaveBeenCalledWith({
      threadId: THREAD_ID,
      messageId: 'agent:req-1',
    });
    // Trimmed after the RPC succeeded: the prompt stays, the reply goes.
    expect(cachedIds(store)).toEqual(['msg_u1']);
  });

  it('derives agent:<requestId> for a msg_ reply that carries its request id', async () => {
    const store = buildStore();
    const { result } = mountAdapter(store);

    await reload(result, 'msg_u2', 'msg_a2');

    expect(hoisted.regenerate).toHaveBeenCalledWith({
      threadId: THREAD_ID,
      messageId: 'agent:req-2',
    });
  });

  it('redoes the last turn (no id) when a msg_ reply is the last assistant message', async () => {
    const store = buildStore();
    const { result } = mountAdapter(store);

    await reload(result, 'msg_u4', 'msg_a4');

    expect(hoisted.regenerate).toHaveBeenCalledWith({ threadId: THREAD_ID, messageId: undefined });
    expect(cachedIds(store)).toEqual(messages.slice(0, 7).map(m => m.id));
  });

  it('drops the last agent reply when the reload carries no ids at all', async () => {
    const store = buildStore();
    const { result } = mountAdapter(store);
    const onReload = result.current.onReload as (
      parentId: string | null,
      config?: { sourceId?: string | null }
    ) => Promise<void>;

    await onReload(null, {});

    expect(hoisted.regenerate).toHaveBeenCalledWith({ threadId: THREAD_ID, messageId: undefined });
    expect(cachedIds(store)).toEqual(messages.slice(0, 7).map(m => m.id));
  });

  it('resolves the reply after parentId when sourceId is absent', async () => {
    const store = buildStore();
    const { result } = mountAdapter(store);
    const onReload = result.current.onReload as (
      parentId: string | null,
      config?: { sourceId?: string | null }
    ) => Promise<void>;

    await onReload('msg_u2', {});

    expect(hoisted.regenerate).toHaveBeenCalledWith({
      threadId: THREAD_ID,
      messageId: 'agent:req-2',
    });
  });

  it('refuses an unresolvable earlier reply without calling the RPC', async () => {
    const store = buildStore();
    const { result } = mountAdapter(store);

    await expect(reload(result, 'msg_u3', 'msg_a3')).resolves.toBeUndefined();

    expect(hoisted.regenerate).not.toHaveBeenCalled();
    expect(cachedIds(store)).toEqual(messages.map(m => m.id));
    expect(hoisted.toastAdd).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
  });

  it('keeps a new reply that streamed in before the RPC resolved', async () => {
    const store = buildStore();
    const { result } = mountAdapter(store);
    // The regenerated turn can finish over the socket before the RPC's own
    // response arrives; trimming "everything after the prompt" then would
    // drop the fresh reply along with the discarded one.
    hoisted.regenerate.mockImplementation(async () => {
      store.dispatch(
        addInferenceResponse.fulfilled(
          { threadId: THREAD_ID, message: row('agent:req-new', 'agent') },
          'request-new',
          {} as never
        )
      );
    });

    await reload(result, 'msg_u4', 'msg_a4');

    expect(cachedIds(store)).toEqual([...messages.slice(0, 7).map(m => m.id), 'agent:req-new']);
  });

  it('keeps the cache and does not reject when the RPC fails', async () => {
    hoisted.regenerate.mockRejectedValue(
      new Error('message msg_u1 is not a regenerable assistant reply')
    );
    const store = buildStore();
    const { result } = mountAdapter(store);

    await expect(reload(result, 'msg_u1', 'agent:req-1')).resolves.toBeUndefined();

    expect(cachedIds(store)).toEqual(messages.map(m => m.id));
    expect(hoisted.toastAdd).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
  });
});
