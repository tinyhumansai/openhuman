import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Thread, ThreadMessage } from '../types/thread';

const mockGetThreads = vi.fn();
const mockGetThreadMessages = vi.fn();

vi.mock('../services/api/threadApi', () => ({
  threadApi: {
    getThreads: () => mockGetThreads(),
    getThreadMessages: (threadId: string) => mockGetThreadMessages(threadId),
  },
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const thread: Thread = {
  id: 'thread-1',
  title: 'Planning',
  chatId: null,
  isActive: true,
  messageCount: 1,
  lastMessageAt: '2026-05-06T00:00:00.000Z',
  createdAt: '2026-05-06T00:00:00.000Z',
  labels: [],
};

const message: ThreadMessage = {
  id: 'message-1',
  content: 'hello',
  type: 'text',
  extraMetadata: {},
  sender: 'user',
  createdAt: '2026-05-06T00:00:00.000Z',
};

describe('useThreadQueries', () => {
  beforeEach(() => {
    mockGetThreads.mockReset();
    mockGetThreadMessages.mockReset();
  });

  it('loads threads with loading and success state', async () => {
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    const { useThreads } = await import('./useThreadQueries');

    const { result } = renderHook(() => useThreads());

    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.data?.threads).toEqual([thread]);
    expect(result.current.data?.count).toBe(1);
    expect(result.current.error).toBeNull();
    expect(mockGetThreads).toHaveBeenCalledTimes(1);
  });

  it('surfaces RPC errors without throwing from render', async () => {
    mockGetThreads.mockRejectedValue(new Error('rpc failed'));
    const { useThreads } = await import('./useThreadQueries');

    const { result } = renderHook(() => useThreads());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.data).toBeNull();
    expect(result.current.error?.message).toBe('rpc failed');
  });

  it('does not load messages when no thread id is available', async () => {
    const { useThreadMessages } = await import('./useThreadQueries');

    const { result, rerender } = renderHook(({ threadId }) => useThreadMessages(threadId), {
      initialProps: { threadId: null as string | null },
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.data).toBeNull();
    expect(result.current.error).toBeNull();
    expect(mockGetThreadMessages).not.toHaveBeenCalled();

    rerender({ threadId: '   ' });
    expect(mockGetThreadMessages).not.toHaveBeenCalled();
  });

  it('loads and refetches thread messages', async () => {
    mockGetThreadMessages
      .mockResolvedValueOnce({ messages: [message], count: 1 })
      .mockResolvedValueOnce({
        messages: [{ ...message, id: 'message-2', content: 'updated' }],
        count: 1,
      });
    const { useThreadMessages } = await import('./useThreadQueries');

    const { result } = renderHook(() => useThreadMessages('thread-1'));

    await waitFor(() => expect(result.current.data?.messages[0].id).toBe('message-1'));
    await act(async () => {
      await result.current.refetch();
    });

    expect(result.current.data?.messages[0].id).toBe('message-2');
    expect(mockGetThreadMessages).toHaveBeenNthCalledWith(1, 'thread-1');
    expect(mockGetThreadMessages).toHaveBeenNthCalledWith(2, 'thread-1');
  });

  it('exposes refetching state while keeping previous data', async () => {
    const nextMessages = deferred<{ messages: ThreadMessage[]; count: number }>();
    mockGetThreadMessages
      .mockResolvedValueOnce({ messages: [message], count: 1 })
      .mockReturnValueOnce(nextMessages.promise);
    const { useThreadMessages } = await import('./useThreadQueries');

    const { result } = renderHook(() => useThreadMessages('thread-1'));

    await waitFor(() => expect(result.current.data?.messages[0].id).toBe('message-1'));

    let refetchPromise: Promise<unknown>;
    act(() => {
      refetchPromise = result.current.refetch();
    });

    expect(result.current.isRefetching).toBe(true);
    expect(result.current.data?.messages[0].id).toBe('message-1');

    nextMessages.resolve({ messages: [{ ...message, id: 'message-2' }], count: 1 });
    await act(async () => {
      await refetchPromise;
    });

    expect(result.current.isRefetching).toBe(false);
    expect(result.current.data?.messages[0].id).toBe('message-2');
  });
});
