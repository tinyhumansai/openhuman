import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('threadApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('loads threads from the threads RPC store', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      data: {
        threads: [
          {
            id: 'default-thread',
            title: 'Conversation',
            chatId: null,
            isActive: true,
            messageCount: 2,
            lastMessageAt: '2026-04-10T12:01:00Z',
            createdAt: '2026-04-10T12:00:00Z',
          },
        ],
        count: 1,
      },
    });

    const { threadApi } = await import('./threadApi');
    const result = await threadApi.getThreads();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.threads_list' });
    expect(result.count).toBe(1);
    expect(result.threads[0].id).toBe('default-thread');
  });

  it('appends a message via threads RPC', async () => {
    const message = {
      id: 'm1',
      content: 'hello',
      type: 'text',
      extraMetadata: {},
      sender: 'user' as const,
      createdAt: '2026-04-10T12:01:00Z',
    };
    mockCallCoreRpc.mockResolvedValueOnce({ data: message });

    const { threadApi } = await import('./threadApi');
    const result = await threadApi.appendMessage('default-thread', message);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.threads_message_append',
      params: { thread_id: 'default-thread', message },
    });
    expect(result).toEqual(message);
  });

  it('generates a thread title via threads RPC', async () => {
    const thread = {
      id: 'default-thread',
      title: 'Invoice follow-up',
      chatId: null,
      isActive: true,
      messageCount: 2,
      lastMessageAt: '2026-04-10T12:01:00Z',
      createdAt: '2026-04-10T12:00:00Z',
    };
    mockCallCoreRpc.mockResolvedValueOnce({ data: thread });

    const { threadApi } = await import('./threadApi');
    const result = await threadApi.generateTitleIfNeeded(
      'default-thread',
      'I can draft the invoice follow-up note for you.'
    );

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.threads_generate_title',
      params: {
        thread_id: 'default-thread',
        assistant_message: 'I can draft the invoice follow-up note for you.',
      },
    });
    expect(result).toEqual(thread);
  });

  it('loads and updates a task board via threads RPC', async () => {
    const taskBoard = {
      threadId: 'thread-1',
      updatedAt: '2026-05-04T10:00:05Z',
      cards: [{ id: 'task-1', title: 'Plan', status: 'todo' as const, order: 0, updatedAt: 'now' }],
    };
    mockCallCoreRpc.mockResolvedValueOnce({ data: { taskBoard } });

    const { threadApi } = await import('./threadApi');
    await expect(threadApi.getTaskBoard('thread-1')).resolves.toEqual(taskBoard);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.threads_task_board_get',
      params: { thread_id: 'thread-1' },
    });

    mockCallCoreRpc.mockResolvedValueOnce({ data: { taskBoard } });
    await expect(threadApi.putTaskBoard('thread-1', taskBoard.cards)).resolves.toEqual(taskBoard);
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.threads_task_board_put',
      params: { thread_id: 'thread-1', cards: taskBoard.cards },
    });
  });

  it('returns null when task board RPC envelopes omit the board', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ data: {} });

    const { threadApi } = await import('./threadApi');
    await expect(threadApi.getTaskBoard('thread-1')).resolves.toBeNull();

    mockCallCoreRpc.mockResolvedValueOnce({ data: {} });
    await expect(threadApi.putTaskBoard('thread-1', [])).resolves.toBeNull();
  });
});
